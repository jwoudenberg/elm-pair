use mvar::MVar;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::ffi::OsStringExt;
use std::os::unix::io::AsRawFd;
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

const VERSION: &str = env!("CARGO_PKG_VERSION");
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
    // Print the socket we're listening on so the editor can connect to it.
    // Immediately flush stdout or we might write to stdout only after
    // daemonization, meaning the socket path would end up in the log instead of
    // being read by the calling editor process.
    std::io::stdout()
        .write_all(socket_path.as_os_str().as_bytes())
        .map_err(|err| {
            log::mk_err!("failed writing socket path to stdout: {:?}", err)
        })?;
    std::io::stdout().flush().map_err(|err| {
        log::mk_err!("failed flushing socket path to stdout: {:?}", err)
    })?;

    // Get an exclusive lock to ensure only one elm-pair is running at a time.
    // Otherwise, every time we start an editor we'll spawn a new elm-pair.
    let did_obtain_lock =
        unsafe { try_obtain_lock(elm_pair_dir.join("lock"))? };
    if !did_obtain_lock {
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
    daemonize(log_file_path)?;

    // Find an Elm compiler for elm-pair to use.
    let compiler = crate::elm::compiler::Compiler::new()?;

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
    let compiler_for_compilation = compiler.clone();
    spawn_thread(analysis_sender.clone(), || {
        compilation_thread::run(
            compilation_receiver,
            analysis_sender,
            compiler_for_compilation,
        )
    });

    // Main thread continues as analysis thread.
    log::info!("elm-pair has started");
    analysis_thread::run(&latest_code, analysis_receiver, compiler)?;
    log::info!("elm-pair exiting");
    Ok(())
}

// Continue running the rest of this program as a daemon. This function follows
// the steps for daemonizing a process outlined in "The Linux Programming
// Interface" (they generalize to other Unix OSes too).
fn daemonize(log_file_path: PathBuf) -> Result<(), Error> {
    // 1: fork()
    match unsafe { libc::fork() } {
        -1 => {
            return Err(log::mk_err!(
                "elm-pair daemonization failed at first fork()"
            ));
        }
        0 => {}
        _child_pid => std::process::exit(0),
    }

    // 2: setsid()
    if unsafe { libc::setsid() } == -1 {
        return Err(log::mk_err!(
            "elm-pair daemonization failed calling setsid()"
        ));
    }

    // 3: fork() again
    match unsafe { libc::fork() } {
        -1 => {
            return Err(log::mk_err!(
                "elm-pair daemonization failed at second fork()"
            ));
        }
        0 => {}
        _child_pid => std::process::exit(0),
    }

    // 4: clear umask
    unsafe { libc::umask(0o077) };

    // 5: set cwd
    std::env::set_current_dir("/").map_err(|err| {
        log::mk_err!("elm-pair daemonization failed setting cwd: {:?}", err)
    })?;

    // 6 and 7: redirect file descriptors
    let stdin = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/null")
        .map_err(|err| {
            log::mk_err!("failed opening /dev/null for writing: {:?}", err)
        })?;
    if unsafe { libc::dup2(stdin.as_raw_fd(), libc::STDIN_FILENO) } == -1 {
        return Err(log::mk_err!(
            "elm-pair daemonization failed redirecting stdin"
        ));
    }

    let log_file = std::fs::File::create(&log_file_path).map_err(|err| {
        log::mk_err!("failed creating log file {:?}: {:?}", log_file_path, err)
    })?;
    if unsafe { libc::dup2(log_file.as_raw_fd(), libc::STDOUT_FILENO) } == -1 {
        return Err(log::mk_err!(
            "elm-pair daemonization failed redirecting stdout"
        ));
    }
    if unsafe { libc::dup2(log_file.as_raw_fd(), libc::STDERR_FILENO) } == -1 {
        return Err(log::mk_err!(
            "elm-pair daemonization failed redirecting stderr"
        ));
    }

    Ok(())
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

// The directory elm-pair uses to store application files. It includes the
// elm-pair version as a cheap means of avoiding versioning trouble, for example
// when upgrading elm-pair or when multiple installed editor plugins use
// different versions of elm-pair concurrently. It's not so nice to run multiple
// elm-pair versions because they will probably waste a bunch of resources on
// calculating/storing the same information, but this seems an unlikely enough
// situation to not invest more work in it for the moment.
fn elm_pair_dir() -> Result<PathBuf, Error> {
    let mut dir = dirs::cache_dir().unwrap_or_else(|| PathBuf::from("/tmp"));
    dir.push("elm-pair");
    dir.push(VERSION);
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
    println!("CODE:\n{}", code.bytes);
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
