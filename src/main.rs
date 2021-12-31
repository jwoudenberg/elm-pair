use mvar::MVar;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex, MutexGuard};
use support::log;
use support::log::Error;
use support::source_code::SourceFileSnapshot;
use tree_sitter::Node;

mod analysis_thread;
mod compilation_thread;
mod editor_listener_thread;
mod editors;
mod elm;
mod support;

#[cfg(test)]
mod test_support;

const MAX_COMPILATION_CANDIDATES: usize = 10;

pub fn main() {
    std::process::exit(match run() {
        Ok(()) => 0,
        Err(err) => {
            log::error!("exiting because of: {:?}", err);
            1
        }
    });
}

fn run() -> Result<(), Error> {
    let elm_pair_dir = elm_pair_dir()?;
    let socket_path = elm_pair_dir.join("socket");
    let socket_path_string = socket_path
        .to_str()
        .ok_or_else(|| {
            log::mk_err!(
                "socket path {:?} contains non-utf8 characters",
                socket_path,
            )
        })?
        .to_owned();
    let print_socket_path = move || print!("{}", socket_path_string);

    // Get an exclusive lock to ensure only one elm-pair is running at a time.
    // Otherwise, every time we start an editor we'll spawn a new elm-pair.
    // TODO: figure out a strategy for dealing with multiple elm-pair versions.
    let did_obtain_lock =
        unsafe { try_obtain_lock(elm_pair_dir.join("lock"))? };
    if !did_obtain_lock {
        print_socket_path();
        return Ok(());
    }

    // Start listening on the socket path. Remove an existing socket file if one
    // was left behind by a previous run (we're past the lock so we're the only
    // running process). We must start listening _before_ we daemonize and exit
    // the main process, because the editor must be able to connect immediately
    // after the main process returns.
    std::fs::remove_file(&socket_path).unwrap_or(());
    let listener = UnixListener::bind(&socket_path).map_err(|err| {
        log::mk_err!("error while creating socket {:?}: {:?}", socket_path, err)
    })?;

    // Fork a daemon process. The main process will exit returning the path to
    // the socket that can be used to communicate with the daemon.
    let log_file_path = elm_pair_dir.join("log");
    let stdout = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_file_path)
        .map_err(|err| {
            log::mk_err!(
                "failed creating log file {:?}: {:?}",
                log_file_path,
                err
            )
        })?;
    let stderr = stdout.try_clone().map_err(|err| {
        log::mk_err!(
            "failed cloning log file handle {:?}: {:?}",
            log_file_path,
            err
        )
    })?;
    let daemonize_result = daemonize::Daemonize::new()
        .stdout(stdout)
        .stderr(stderr)
        .exit_action(print_socket_path)
        .start();
    match daemonize_result {
        Ok(()) => {
            // We're continuing as an elm-pair daemon.
        }
        Err(err) => {
            // An unexpected error happened.
            return Err(log::mk_err!("failed starting daemon: {:?}", err));
        }
    }

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
            listener,
            latest_code_for_editor_listener,
            compilation_sender,
            analysis_sender_for_editor_listener,
        )
    });

    // Start compilation thread.
    spawn_thread(analysis_sender.clone(), || {
        compilation_thread::run(compilation_receiver, analysis_sender)
    });

    log::info!("elm-pair has started");

    // Main thread continues as analysis thread.
    analysis_thread::run(&latest_code, analysis_receiver)
}

// Obtain a file lock on a Unix system. No safe API exists for this in the
// standard library.
unsafe fn try_obtain_lock(path: PathBuf) -> Result<bool, Error> {
    let path_c = std::ffi::CString::new(path.into_os_string().into_vec())
        .map_err(|_| log::mk_err!("Path contained nul byte"))?;

    let fd = libc::open(path_c.as_ptr(), libc::O_WRONLY | libc::O_CREAT, 0o666);
    if fd == -1 {
        return Err(log::mk_err!("Could not open lockfile"));
    }
    let res = libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB);
    Ok(res != -1)
}

fn elm_pair_dir() -> Result<PathBuf, Error> {
    let cache_dir = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    let dir = cache_dir.join("elm-pair");
    std::fs::create_dir_all(&dir).map_err(|err| {
        log::mk_err!("error while creating directory {:?}: {:?}", dir, err)
    })?;
    Ok(dir)
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
