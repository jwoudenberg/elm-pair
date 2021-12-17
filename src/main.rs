use mvar::MVar;
use std::sync::mpsc::{Receiver, SendError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use support::source_code::{Buffer, SourceFileSnapshot};
use tree_sitter::Node;

mod analysis_thread;
mod compilation_thread;
mod editor_listener_thread;
mod editors;
mod languages;
mod support;

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

fn run() -> Result<(), Error> {
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

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<T> {
    // `mutex.lock()` only fails if the lock is 'poisoned', meaning another
    // thread panicked while accessing it. In this program we have no intent
    // to recover from panicked threads, so letting the original problem
    // showball by calling `unwrap()` here is fine.
    mutex.lock().unwrap()
}

// TODO: review these errors and see which ones we can handle without crashing.
#[derive(Debug)]
pub(crate) enum Error {
    NoElmJsonFoundInAnyAncestorDirectory,
    CouldNotReadCurrentWorkingDirectory(std::io::Error),
    DidNotFindPathEnvVar,
    FailedToSendMessage,
    SocketCreationFailed(std::io::Error),
    AcceptingIncomingSocketConnectionFailed(std::io::Error),
    CloningSocketFailed(std::io::Error),

    // Elm errors
    ElmDidNotFindCompilerBinaryOnPath,
    ElmCompilationFailedToCreateTempDir(std::io::Error),
    ElmCompilationFailedToWriteCodeToTempFile(std::io::Error),
    ElmCompilationFailedToRunElmMake(std::io::Error),
    ElmNoProjectStoredForBuffer(Buffer),
    ElmNoSuchTypeInModule {
        type_name: String,
        module_name: String,
    },
    ElmIdatReadingFailed(std::io::Error),
    ElmIdatReadingUtf8Failed(std::str::Utf8Error),
    ElmIdatReadingUnexpectedKind {
        kind: u8,
        during: String,
    },
    ElmCannotQualifyOperator(String),

    // Treesitter errors
    TreeSitterParsingFailed,
    TreeSitterSettingLanguageFailed(tree_sitter::LanguageError),
    TreeSitterExpectedNodeDoesNotExist,
    TreeSitterFailedToParseQuery(tree_sitter::QueryError),
    TreeSitterQueryReturnedNotEnoughMatches,
    TreeSitterQueryDoesNotHaveExpectedIndex,

    // Neovim errors
    NeovimEncodingFailedWhileWritingMarker(std::io::Error),
    NeovimEncodingFailedWhileWritingData(std::io::Error),
    NeovimEncodingFailedWhileWritingString(std::io::Error),
    NeovimUnknownMessageType(u32, u8),
    NeovimUnknownEventMethod(String),
    NeovimReceivedErrorEvent(u64, String),
    NeovimReceivedLinesEventForUnknownBuffer(Buffer),
    NeovimDecodingReadingMarker(std::io::Error),
    NeovimDecodingReadingData(std::io::Error),
    NeovimDecodingSkippingData(std::io::Error),
    NeovimDecodingReadingString(std::io::Error),
    NeovimDecodingTypeMismatch(rmp::Marker),
    NeovimDecodingOutOfRange,
    NeovimDecodingInvalidUtf8(core::str::Utf8Error),
    NeovimDecodingBufferCannotHoldString(u32),
    NeovimDecodingNotEnoughArrayElements {
        expected: u32,
        actual: u32,
    },
}

impl From<rmp::encode::ValueWriteError> for Error {
    fn from(error: rmp::encode::ValueWriteError) -> Error {
        match error {
            rmp::encode::ValueWriteError::InvalidMarkerWrite(sub_error) => {
                Error::NeovimEncodingFailedWhileWritingMarker(sub_error)
            }
            rmp::encode::ValueWriteError::InvalidDataWrite(sub_error) => {
                Error::NeovimEncodingFailedWhileWritingData(sub_error)
            }
        }
    }
}

impl From<rmp::decode::MarkerReadError> for Error {
    fn from(
        rmp::decode::MarkerReadError(error): rmp::decode::MarkerReadError,
    ) -> Error {
        Error::NeovimDecodingReadingMarker(error)
    }
}

impl From<rmp::decode::ValueReadError> for Error {
    fn from(error: rmp::decode::ValueReadError) -> Error {
        match error {
            rmp::decode::ValueReadError::InvalidMarkerRead(sub_error) => {
                Error::NeovimDecodingReadingMarker(sub_error)
            }
            rmp::decode::ValueReadError::InvalidDataRead(sub_error) => {
                Error::NeovimDecodingReadingData(sub_error)
            }
            rmp::decode::ValueReadError::TypeMismatch(sub_error) => {
                Error::NeovimDecodingTypeMismatch(sub_error)
            }
        }
    }
}

impl From<rmp::decode::NumValueReadError> for Error {
    fn from(error: rmp::decode::NumValueReadError) -> Error {
        match error {
            rmp::decode::NumValueReadError::InvalidMarkerRead(sub_error) => {
                Error::NeovimDecodingReadingMarker(sub_error)
            }
            rmp::decode::NumValueReadError::InvalidDataRead(sub_error) => {
                Error::NeovimDecodingReadingData(sub_error)
            }
            rmp::decode::NumValueReadError::TypeMismatch(sub_error) => {
                Error::NeovimDecodingTypeMismatch(sub_error)
            }
            rmp::decode::NumValueReadError::OutOfRange => {
                Error::NeovimDecodingOutOfRange
            }
        }
    }
}

impl From<rmp::decode::DecodeStringError<'_>> for Error {
    fn from(error: rmp::decode::DecodeStringError) -> Error {
        match error {
            rmp::decode::DecodeStringError::InvalidMarkerRead(sub_error) => {
                Error::NeovimDecodingReadingMarker(sub_error)
            }
            rmp::decode::DecodeStringError::InvalidDataRead(sub_error) => {
                Error::NeovimDecodingReadingData(sub_error)
            }
            rmp::decode::DecodeStringError::TypeMismatch(sub_error) => {
                Error::NeovimDecodingTypeMismatch(sub_error)
            }
            rmp::decode::DecodeStringError::BufferSizeTooSmall(sub_error) => {
                Error::NeovimDecodingBufferCannotHoldString(sub_error)
            }
            rmp::decode::DecodeStringError::InvalidUtf8(_, sub_error) => {
                Error::NeovimDecodingInvalidUtf8(sub_error)
            }
        }
    }
}

impl<T> From<SendError<T>> for Error {
    fn from(_err: SendError<T>) -> Error {
        Error::FailedToSendMessage
    }
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
        node.kind_id(),
        code.slice(&node.byte_range()).to_string(),
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
