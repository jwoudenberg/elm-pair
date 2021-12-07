use core::ops::Range;
use mvar::MVar;
use ropey::Rope;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use tree_sitter::{InputEdit, Node, Tree};

mod analysis_thread;
mod compilation_thread;
mod editor_listener_thread;
mod editors;

#[cfg(test)]
mod test_support;

const MAX_COMPILATION_CANDIDATES: usize = 10;

pub fn main() {
    std::process::exit(match run() {
        Ok(()) => 0,
        Err(err) => {
            eprintln!("[fatal] encountered unexpected error: {:?}", err);
            1
        }
    });
}

fn run() -> Result<(), analysis_thread::Error> {
    // Create channels for inter-thread communication.
    let (analysis_sender, analysis_receiver) = std::sync::mpsc::channel();
    let (compilation_sender, compilation_receiver) = std::sync::mpsc::channel();
    // We could send code updates over above channels too, but don't because:
    // 1. It would require cloning a snapshot on every change, which is often.
    // 2. By using a mutex we can block analysis of a snapshot currently being
    //    changed, meaning we already know it's no longer current.
    let latest_code = Arc::new(MVar::new_empty());

    // Start editor listener thread.
    let latest_code_for_editor_listener = latest_code.clone();
    let analysis_sender_for_editor_listener = analysis_sender.clone();
    spawn_thread(analysis_sender.clone(), || {
        editor_listener_thread::run(
            latest_code_for_editor_listener,
            compilation_sender,
            analysis_sender_for_editor_listener,
        )
    });

    // Start compilation thread.
    spawn_thread(analysis_sender.clone(), || {
        compilation_thread::run(compilation_receiver, analysis_sender)
    });

    // Main thread continues as analysis thread.
    analysis_thread::run(&latest_code, analysis_receiver)
}

#[derive(Clone)]
struct SourceFileSnapshot {
    // A unique index identifying a source file open in an editor. We're not
    // using the file path for a couple of reasons:
    // - It's possible for the same file to be open in multiple editors with
    //   different unsaved changes each.
    // - A file path is stringy, so more expensive to copy.
    buffer: Buffer,
    // The full contents of the file, stored in a Rope datastructure. This
    // datastructure offers cheap modifications in random locations, and cheap
    // cloning (both of which we'll do a lot).
    bytes: Rope,
    // The tree-sitter concrete syntax tree representing the code in `bytes`.
    // This tree by itself is not enough to recover the source code, which is
    // why we also keep the original source code in `bytes`.
    tree: Tree,
    // A number that gets incremented for each change to this snapshot.
    revision: usize,
}

impl<'a> tree_sitter::TextProvider<'a> for &'a SourceFileSnapshot {
    type I = Chunks<'a>;

    fn text(&mut self, node: Node<'_>) -> Chunks<'a> {
        let range = node.byte_range();
        let (chunks, first_chunk_start_byte, _, _) =
            self.bytes.chunks_at_byte(range.start);
        Chunks {
            chunks,
            skip_start: range.start - first_chunk_start_byte,
            remaining: range.len(),
        }
    }
}

struct Chunks<'a> {
    chunks: ropey::iter::Chunks<'a>,
    skip_start: usize,
    remaining: usize,
}

impl<'a> Iterator for Chunks<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.remaining > 0 {
            let chunk = self.chunks.next()?;
            let slice = chunk
                [self.skip_start..std::cmp::min(self.remaining, chunk.len())]
                .as_bytes();
            self.skip_start = 0;
            self.remaining -= slice.len();
            Some(slice)
        } else {
            None
        }
    }
}

// A unique identifier for a buffer that elm-pair is tracking in any connected
// editor. First 32 bits uniquely identify the connected editor, while the last
// 32 bits identify one of the buffers openen in that particular editor.
#[derive(Copy, Clone, Debug, Hash, PartialEq)]
struct Buffer {
    editor_id: u32,
    buffer_id: u32,
}

impl Eq for Buffer {}

// A change made by the user reported by the editor.
#[derive(Debug)]
struct Edit {
    // The buffer that was changed.
    buffer: Buffer,
    // A tree-sitter InputEdit value, describing what part of the file was changed.
    input_edit: InputEdit,
    // Bytes representing the new contents of the file at the location described
    // by `input_edit`.
    new_bytes: String,
}

fn byte_to_point(code: &Rope, byte: usize) -> tree_sitter::Point {
    let row = code.byte_to_line(byte);
    tree_sitter::Point {
        row,
        column: code.byte_to_char(byte) - code.line_to_char(row),
    }
}

fn spawn_thread<M, E, F>(error_channel: Sender<M>, f: F)
where
    M: Send + 'static,
    F: FnOnce() -> Result<(), E>,
    E: Into<M>,
    F: Send + 'static,
{
    std::thread::spawn(move || {
        match f() {
            Ok(_) => {}
            Err(err) => {
                error_channel
                    .send(err.into())
                    // If sending fails there's nothing more we can do to report
                    // this error, hence the unwrap().
                    .unwrap();
            }
        }
    });
}

fn debug_code_slice(code: &SourceFileSnapshot, range: &Range<usize>) -> String {
    let start = code.bytes.byte_to_char(range.start);
    let end = code.bytes.byte_to_char(range.end);
    code.bytes.slice(start..end).to_string()
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<T> {
    // `mutex.lock()` only fails if the lock is 'poisoned', meaning another
    // thread panicked while accessing it. In this program we have no intent
    // to recover from panicked threads, so letting the original problem
    // showball by calling `unwrap()` here is fine.
    mutex.lock().unwrap()
}

// TODO: remove debug helper when it's no longer needed.
#[allow(dead_code)]
fn debug_print_code(code: &SourceFileSnapshot) {
    println!("CODE:\n{}", code.bytes.to_string());
}

// TODO: remove debug helper when it's no longer needed.
#[allow(dead_code)]
fn debug_print_tree(code: &SourceFileSnapshot) {
    let mut cursor = code.tree.walk();
    debug_print_tree_helper(code, 0, &mut cursor);
    println!();
}

fn debug_print_tree_helper(
    code: &SourceFileSnapshot,
    indent: usize,
    cursor: &mut tree_sitter::TreeCursor,
) {
    let node = cursor.node();
    debug_print_node(code, indent, &node);
    if cursor.goto_first_child() {
        debug_print_tree_helper(code, indent + 1, cursor);
        cursor.goto_parent();
    }
    if cursor.goto_next_sibling() {
        debug_print_tree_helper(code, indent, cursor);
    }
}

fn debug_print_node(code: &SourceFileSnapshot, indent: usize, node: &Node) {
    println!(
        "{}[{} {:?}] {:?}{}",
        "  ".repeat(indent),
        node.kind(),
        node.start_position().row + 1,
        debug_code_slice(code, &node.byte_range()),
        if node.has_changes() { " (changed)" } else { "" },
    );
}

trait MsgLoop<E> {
    type Msg;

    // This function is called for every new message that arrives. If we return
    // a `false` value at any point we stop the loop.
    //
    // This function doesn't return until it has processed at least one message,
    // and then until it has emptied the current contents of the queue.
    fn on_msg(&mut self, msg: Self::Msg) -> Result<bool, E>;

    // After each batch of messages this function is called once to do other
    // work. After it returns we wait for more messages.
    fn on_idle(&mut self) -> Result<(), E>;

    // --- Implementation, not for overriding ---
    fn start(&mut self, mut receiver: Receiver<Self::Msg>) -> Result<(), E> {
        while self.process_msg_batch(&mut receiver)? {
            self.on_idle()?;
        }
        Ok(())
    }

    fn process_msg_batch(
        &mut self,
        receiver: &mut Receiver<Self::Msg>,
    ) -> Result<bool, E> {
        match self.process_msg_batch_helper(receiver) {
            Ok(res) => res,
            Err(TryRecvError::Empty) => Ok(true),
            Err(TryRecvError::Disconnected) => Ok(false),
        }
    }

    fn process_msg_batch_helper(
        &mut self,
        receiver: &mut Receiver<Self::Msg>,
    ) -> Result<Result<bool, E>, TryRecvError> {
        let mut msg = receiver.recv()?;
        loop {
            let res = self.on_msg(msg);
            match res {
                Ok(do_continue) => {
                    if !do_continue {
                        return Ok(res);
                    }
                }
                Err(_) => return Ok(res),
            }
            msg = receiver.try_recv()?;
        }
    }
}

// A thread sync structure similar to Haskell's MVar. A variable, potentially
// empty, that can be shared across threads. Doesn't (currently) do blocking
// reads and writes though, because this codebase doesn't need it.
mod mvar {
    use crate::lock;
    use std::sync::Mutex;

    pub struct MVar<T> {
        val: Mutex<Option<T>>,
    }

    impl<T> MVar<T> {
        pub fn new_empty() -> MVar<T> {
            MVar {
                val: Mutex::new(None),
            }
        }

        // Write a value to the MVar. If the MVar already contained a value, it
        // is returned.
        pub fn replace(&self, new: T) -> Option<T> {
            let mut val = lock(&self.val);
            val.replace(new)
        }

        // Take the value from an MVar if it has one, leaving the MVar empty.
        pub fn try_take(&self) -> Option<T> {
            lock(&self.val).take()
        }

        // Clone the current value in the MVar and return it.
        pub fn try_read(&self) -> Option<T>
        where
            T: Clone,
        {
            crate::lock(&self.val).clone()
        }
    }
}

// A stack (last in, first out) with a maximum size. If a push would ever make
// the stack grow beyond its capacity, then the stack forgets its oldest element
// before pushing the new element.
mod sized_stack {
    use std::collections::VecDeque;

    pub struct SizedStack<T> {
        capacity: usize,
        items: VecDeque<T>,
    }

    impl<T> SizedStack<T> {
        pub fn with_capacity(capacity: usize) -> SizedStack<T> {
            SizedStack {
                capacity,
                items: VecDeque::with_capacity(capacity),
            }
        }

        // Push an item on the stack.
        pub fn push(&mut self, item: T) {
            self.items.truncate(self.capacity - 1);
            self.items.push_front(item);
        }

        // Pop an item of the stack. This function blocks until an item becomes
        // available.
        pub fn pop(&mut self) -> Option<T> {
            self.items.pop_front()
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::test_support::simulation::simulation_test;

    simulation_test!(change_record_field_name);
    simulation_test!(add_field_to_record);
    simulation_test!(add_field_to_front_of_record);
    simulation_test!(add_field_to_empty_record);
    simulation_test!(remove_field_from_record);
    simulation_test!(remove_only_field_from_record);
    simulation_test!(remove_field_from_front_of_record);
    simulation_test!(add_import);
    simulation_test!(remove_import);
    simulation_test!(add_as_clause_to_import);
    simulation_test!(remove_as_clause_from_import);
    simulation_test!(change_as_clause_on_import);
    simulation_test!(change_argument_name_at_definition_site);
    simulation_test!(change_argument_name_at_usage_site);
    simulation_test!(change_let_binding_name_at_definition_site);
    simulation_test!(change_let_binding_name_at_usage_site);
    simulation_test!(change_function_name_in_type_definition);
    simulation_test!(change_function_name_at_definition_site);
    simulation_test!(change_function_name_at_usage_site);
    simulation_test!(change_type_name_at_definition_site);
    simulation_test!(change_type_name_at_usage_site);
    simulation_test!(add_type_definition);
    simulation_test!(remove_type_definition);
    simulation_test!(add_type_alias_definition);
    simulation_test!(remove_type_alias_definition);
    simulation_test!(add_module_qualifier_to_variable);
    simulation_test!(remove_module_qualifier_from_variable);
    simulation_test!(add_module_qualifier_to_type);
    simulation_test!(remove_module_qualifier_from_type);
    simulation_test!(no_interpretation_when_back_at_compiling_state);
}
