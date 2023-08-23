#[cfg(feature = "mmap")]
extern crate memmap2;

use std::error;
use std::fmt;
use std::fs;
use std::io::prelude::*;
use std::io::ErrorKind;
use std::ops::Deref;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
#[cfg(windows)]
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time;

static COUNTER: AtomicUsize = AtomicUsize::new(0);

use mailparse::*;

#[cfg(unix)]
const INFORMATIONAL_SUFFIX_SEPARATOR: &str = ":";
#[cfg(windows)]
const INFORMATIONAL_SUFFIX_SEPARATOR: &str = ";";

#[derive(Debug)]
pub enum MailEntryError {
    IOError(std::io::Error),
    ParseError(MailParseError),
    DateError(&'static str),
}

impl fmt::Display for MailEntryError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match *self {
            MailEntryError::IOError(ref err) => write!(f, "IO error: {}", err),
            MailEntryError::ParseError(ref err) => write!(f, "Parse error: {}", err),
            MailEntryError::DateError(ref msg) => write!(f, "Date error: {}", msg),
        }
    }
}

impl error::Error for MailEntryError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match *self {
            MailEntryError::IOError(ref err) => Some(err),
            MailEntryError::ParseError(ref err) => Some(err),
            MailEntryError::DateError(_) => None,
        }
    }
}

impl From<std::io::Error> for MailEntryError {
    fn from(err: std::io::Error) -> MailEntryError {
        MailEntryError::IOError(err)
    }
}

impl From<MailParseError> for MailEntryError {
    fn from(err: MailParseError) -> MailEntryError {
        MailEntryError::ParseError(err)
    }
}

impl From<&'static str> for MailEntryError {
    fn from(err: &'static str) -> MailEntryError {
        MailEntryError::DateError(err)
    }
}

enum MailData {
    None,
    #[cfg(not(feature = "mmap"))]
    Bytes(Vec<u8>),
    #[cfg(feature = "mmap")]
    File(memmap2::Mmap),
}

impl MailData {
    fn is_none(&self) -> bool {
        match self {
            MailData::None => true,
            _ => false,
        }
    }
}

/// This struct represents a single email message inside
/// the maildir. Creation of the struct does not automatically
/// load the content of the email file into memory - however,
/// that may happen upon calling functions that require parsing
/// the email.
pub struct MailEntry {
    id: String,
    flags: String,
    path: PathBuf,
    data: MailData,
}

impl MailEntry {
    pub fn id(&self) -> &str {
        &self.id
    }

    fn read_data(&mut self) -> std::io::Result<()> {
        if self.data.is_none() {
            #[cfg(feature = "mmap")]
            {
                let f = fs::File::open(&self.path)?;
                let mmap = unsafe { memmap2::MmapOptions::new().map(&f)? };
                self.data = MailData::File(mmap);
            }

            #[cfg(not(feature = "mmap"))]
            {
                let mut f = fs::File::open(&self.path)?;
                let mut d = Vec::<u8>::new();
                f.read_to_end(&mut d)?;
                self.data = MailData::Bytes(d);
            }
        }
        Ok(())
    }

    pub fn parsed(&mut self) -> Result<ParsedMail, MailEntryError> {
        self.read_data()?;
        match self.data {
            MailData::None => panic!("read_data should have returned an Err!"),
            #[cfg(not(feature = "mmap"))]
            MailData::Bytes(ref b) => parse_mail(b).map_err(MailEntryError::ParseError),
            #[cfg(feature = "mmap")]
            MailData::File(ref m) => parse_mail(m).map_err(MailEntryError::ParseError),
        }
    }

    pub fn headers(&mut self) -> Result<Vec<MailHeader>, MailEntryError> {
        self.read_data()?;
        let headers = match self.data {
            MailData::None => panic!("read_data should have returned an Err!"),
            #[cfg(not(feature = "mmap"))]
            MailData::Bytes(ref b) => parse_headers(b),
            #[cfg(feature = "mmap")]
            MailData::File(ref m) => parse_headers(m),
        };
        headers.map(|(v, _)| v).map_err(MailEntryError::ParseError)
    }

    pub fn received(&mut self) -> Result<i64, MailEntryError> {
        self.read_data()?;
        let headers = self.headers()?;
        let received = headers.get_first_value("Received");
        match received {
            Some(v) => v
                .rsplit(';')
                .nth(0)
                .ok_or_else(|| MailEntryError::DateError("Unable to split Received header"))
                .and_then(|ts| dateparse(ts).map_err(MailEntryError::from)),
            None => Err("No Received header found")?,
        }
    }

    pub fn date(&mut self) -> Result<i64, MailEntryError> {
        self.read_data()?;
        let headers = self.headers()?;
        let date = headers.get_first_value("Date");
        match date {
            Some(ts) => dateparse(&ts).map_err(MailEntryError::from),
            None => Err("No Date header found")?,
        }
    }

    pub fn flags(&self) -> &str {
        &self.flags
    }

    pub fn is_draft(&self) -> bool {
        self.flags.contains('D')
    }

    pub fn is_flagged(&self) -> bool {
        self.flags.contains('F')
    }

    pub fn is_passed(&self) -> bool {
        self.flags.contains('P')
    }

    pub fn is_replied(&self) -> bool {
        self.flags.contains('R')
    }

    pub fn is_seen(&self) -> bool {
        self.flags.contains('S')
    }

    pub fn is_trashed(&self) -> bool {
        self.flags.contains('T')
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }
}

enum Subfolder {
    New,
    Cur,
}

/// An iterator over the email messages in a particular
/// maildir subfolder (either `cur` or `new`). This iterator
/// produces a `std::io::Result<MailEntry>`, which can be an
/// `Err` if an error was encountered while trying to read
/// file system properties on a particular entry, or if an
/// invalid file was found in the maildir. Files starting with
/// a dot (.) character in the maildir folder are ignored.
pub struct MailEntries {
    path: PathBuf,
    subfolder: Subfolder,
    readdir: Option<fs::ReadDir>,
}

impl MailEntries {
    fn new(path: PathBuf, subfolder: Subfolder) -> MailEntries {
        MailEntries {
            path,
            subfolder,
            readdir: None,
        }
    }
}

impl Iterator for MailEntries {
    type Item = std::io::Result<MailEntry>;

    fn next(&mut self) -> Option<std::io::Result<MailEntry>> {
        if self.readdir.is_none() {
            let mut dir_path = self.path.clone();
            dir_path.push(match self.subfolder {
                Subfolder::New => "new",
                Subfolder::Cur => "cur",
            });
            self.readdir = match fs::read_dir(dir_path) {
                Err(_) => return None,
                Ok(v) => Some(v),
            };
        }

        loop {
            // we need to skip over files starting with a '.'
            let dir_entry = self.readdir.iter_mut().next().unwrap().next();
            let result = dir_entry.map(|e| {
                let entry = e?;
                let filename = String::from(entry.file_name().to_string_lossy().deref());
                if filename.starts_with('.') {
                    return Ok(None);
                }
                let (id, flags) = match self.subfolder {
                    Subfolder::New => (Some(filename.as_str()), Some("")),
                    Subfolder::Cur => {
                        let delim = format!("{}2,", INFORMATIONAL_SUFFIX_SEPARATOR);
                        let mut iter = filename.split(&delim);
                        (iter.next(), iter.next())
                    }
                };
                if id.is_none() || flags.is_none() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Non-maildir file found in maildir",
                    ));
                }
                Ok(Some(MailEntry {
                    id: String::from(id.unwrap()),
                    flags: String::from(flags.unwrap()),
                    path: entry.path(),
                    data: MailData::None,
                }))
            });
            return match result {
                None => None,
                Some(Err(e)) => Some(Err(e)),
                Some(Ok(None)) => continue,
                Some(Ok(Some(v))) => Some(Ok(v)),
            };
        }
    }
}

#[derive(Debug)]
pub enum MaildirError {
    Io(std::io::Error),
    Utf8(std::str::Utf8Error),
    Time(std::time::SystemTimeError),
}

impl fmt::Display for MaildirError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        use MaildirError::*;

        match *self {
            Io(ref e) => write!(f, "IO Error: {}", e),
            Utf8(ref e) => write!(f, "UTF8 Encoding Error: {}", e),
            Time(ref e) => write!(f, "Time Error: {}", e),
        }
    }
}

impl error::Error for MaildirError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        use MaildirError::*;

        match *self {
            Io(ref e) => Some(e),
            Utf8(ref e) => Some(e),
            Time(ref e) => Some(e),
        }
    }
}

impl From<std::io::Error> for MaildirError {
    fn from(e: std::io::Error) -> MaildirError {
        MaildirError::Io(e)
    }
}
impl From<std::str::Utf8Error> for MaildirError {
    fn from(e: std::str::Utf8Error) -> MaildirError {
        MaildirError::Utf8(e)
    }
}
impl From<std::time::SystemTimeError> for MaildirError {
    fn from(e: std::time::SystemTimeError) -> MaildirError {
        MaildirError::Time(e)
    }
}

/// An iterator over the maildir subdirectories. This iterator
/// produces a `std::io::Result<Maildir>`, which can be an
/// `Err` if an error was encountered while trying to read
/// file system properties on a particular entry. Only
/// subdirectories starting with a single period are included.
pub struct MaildirEntries {
    path: PathBuf,
    readdir: Option<fs::ReadDir>,
}

impl MaildirEntries {
    fn new(path: PathBuf) -> MaildirEntries {
        MaildirEntries {
            path,
            readdir: None,
        }
    }
}

impl Iterator for MaildirEntries {
    type Item = std::io::Result<Maildir>;

    fn next(&mut self) -> Option<std::io::Result<Maildir>> {
        if self.readdir.is_none() {
            self.readdir = match fs::read_dir(&self.path) {
                Err(_) => return None,
                Ok(v) => Some(v),
            };
        }

        loop {
            let dir_entry = self.readdir.iter_mut().next().unwrap().next();
            let result = dir_entry.map(|e| {
                let entry = e?;

                // a dir name should start by one single period
                let filename = String::from(entry.file_name().to_string_lossy().deref());
                if !filename.starts_with('.') || filename.starts_with("..") {
                    return Ok(None);
                }

                // the entry should be a directory
                let is_dir = entry.metadata().map(|m| m.is_dir()).unwrap_or_default();
                if !is_dir {
                    return Ok(None);
                }

                Ok(Some(Maildir {
                    path: self.path.join(filename),
                }))
            });

            return match result {
                None => None,
                Some(Err(e)) => Some(Err(e)),
                Some(Ok(None)) => continue,
                Some(Ok(Some(v))) => Some(Ok(v)),
            };
        }
    }
}

/// The main entry point for this library. This struct can be
/// instantiated from a path using the `from` implementations.
/// The path passed in to the `from` should be the root of the
/// maildir (the folder containing `cur`, `new`, and `tmp`).
pub struct Maildir {
    path: PathBuf,
}

impl Maildir {
    /// Returns the path of the maildir base folder.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Returns the number of messages found inside the `new`
    /// maildir folder.
    pub fn count_new(&self) -> usize {
        self.list_new().count()
    }

    /// Returns the number of messages found inside the `cur`
    /// maildir folder.
    pub fn count_cur(&self) -> usize {
        self.list_cur().count()
    }

    /// Returns an iterator over the messages inside the `new`
    /// maildir folder. The order of messages in the iterator
    /// is not specified, and is not guaranteed to be stable
    /// over multiple invocations of this method.
    pub fn list_new(&self) -> MailEntries {
        MailEntries::new(self.path.clone(), Subfolder::New)
    }

    /// Returns an iterator over the messages inside the `cur`
    /// maildir folder. The order of messages in the iterator
    /// is not specified, and is not guaranteed to be stable
    /// over multiple invocations of this method.
    pub fn list_cur(&self) -> MailEntries {
        MailEntries::new(self.path.clone(), Subfolder::Cur)
    }

    /// Returns an iterator over the maildir subdirectories.
    /// The order of subdirectories in the iterator
    /// is not specified, and is not guaranteed to be stable
    /// over multiple invocations of this method.
    pub fn list_subdirs(&self) -> MaildirEntries {
        MaildirEntries::new(self.path.clone())
    }

    /// Moves a message from the `new` maildir folder to the
    /// `cur` maildir folder. The id passed in should be
    /// obtained from the iterator produced by `list_new`.
    pub fn move_new_to_cur(&self, id: &str) -> std::io::Result<()> {
        self.move_new_to_cur_with_flags(id, "")
    }

    /// Moves a message from the `new` maildir folder to the `cur` maildir folder, and sets the
    /// given flags. The id passed in should be obtained from the iterator produced by `list_new`.
    ///
    /// The possible flags are described e.g. at <https://cr.yp.to/proto/maildir.html> or
    /// <http://www.courier-mta.org/maildir.html>.
    pub fn move_new_to_cur_with_flags(&self, id: &str, flags: &str) -> std::io::Result<()> {
        let src = self.path.join("new").join(id);
        let dst = self.path.join("cur").join(format!(
            "{}{}2,{}",
            id,
            INFORMATIONAL_SUFFIX_SEPARATOR,
            Self::normalize_flags(flags)
        ));
        fs::rename(src, dst)
    }

    /// Copies a message from the current maildir to the targetted maildir.
    pub fn copy_to(&self, id: &str, target: &Maildir) -> std::io::Result<()> {
        let entry = self.find(id).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Mail entry not found")
        })?;
        let filename = entry.path().file_name().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid mail entry file name",
            )
        })?;

        let src_path = entry.path();
        let dst_path = target.path().join("cur").join(filename);
        if src_path == &dst_path {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Target maildir needs to be different from the source",
            ));
        }

        fs::copy(src_path, dst_path)?;
        Ok(())
    }

    /// Moves a message from the current maildir to the targetted maildir.
    pub fn move_to(&self, id: &str, target: &Maildir) -> std::io::Result<()> {
        let entry = self.find(id).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Mail entry not found")
        })?;
        let filename = entry.path().file_name().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Invalid mail entry file name",
            )
        })?;
        fs::rename(entry.path(), target.path().join("cur").join(filename))?;
        Ok(())
    }

    /// Tries to find the message with the given id in the
    /// maildir. This searches both the `new` and the `cur`
    /// folders.
    pub fn find(&self, id: &str) -> Option<MailEntry> {
        let filter = |entry: &std::io::Result<MailEntry>| match *entry {
            Err(_) => false,
            Ok(ref e) => e.id() == id,
        };

        self.list_new()
            .find(&filter)
            .or_else(|| self.list_cur().find(&filter))
            .map(|e| e.unwrap())
    }

    fn normalize_flags(flags: &str) -> String {
        let mut flag_chars = flags.chars().collect::<Vec<char>>();
        flag_chars.sort();
        flag_chars.dedup();
        flag_chars.into_iter().collect()
    }

    fn update_flags<F>(&self, id: &str, flag_op: F) -> std::io::Result<()>
    where
        F: Fn(&str) -> String,
    {
        let filter = |entry: &std::io::Result<MailEntry>| match *entry {
            Err(_) => false,
            Ok(ref e) => e.id() == id,
        };

        match self.list_cur().find(&filter).map(|e| e.unwrap()) {
            Some(m) => {
                let src = m.path();
                let mut dst = m.path().clone();
                dst.pop();
                dst.push(format!(
                    "{}{}2,{}",
                    m.id(),
                    INFORMATIONAL_SUFFIX_SEPARATOR,
                    flag_op(m.flags())
                ));
                fs::rename(src, dst)
            }
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Mail entry not found",
            )),
        }
    }

    /// Updates the flags for the message with the given id in the
    /// maildir. This only searches the `cur` folder, because that's
    /// the folder where messages have flags. Returns an error if the
    /// message was not found. All existing flags are overwritten with
    /// the new flags provided.
    pub fn set_flags(&self, id: &str, flags: &str) -> std::io::Result<()> {
        self.update_flags(id, |_old_flags| Self::normalize_flags(flags))
    }

    /// Adds the given flags to the message with the given id in the maildir.
    /// This only searches the `cur` folder, because that's the folder where
    /// messages have flags. Returns an error if the message was not found.
    /// Flags are deduplicated, so setting a already-set flag has no effect.
    pub fn add_flags(&self, id: &str, flags: &str) -> std::io::Result<()> {
        let flag_merge = |old_flags: &str| {
            let merged = String::from(old_flags) + flags;
            Self::normalize_flags(&merged)
        };
        self.update_flags(id, &flag_merge)
    }

    /// Removes the given flags to the message with the given id in the maildir.
    /// This only searches the `cur` folder, because that's the folder where
    /// messages have flags. Returns an error if the message was not found.
    /// If the message doesn't have the flag(s) to be removed, those flags are
    /// ignored.
    pub fn remove_flags(&self, id: &str, flags: &str) -> std::io::Result<()> {
        let flag_strip =
            |old_flags: &str| old_flags.chars().filter(|c| !flags.contains(*c)).collect();
        self.update_flags(id, &flag_strip)
    }

    /// Deletes the message with the given id in the maildir.
    /// This searches both the `new` and the `cur` folders,
    /// and deletes the file from the filesystem. Returns an
    /// error if no message was found with the given id.
    pub fn delete(&self, id: &str) -> std::io::Result<()> {
        match self.find(id) {
            Some(m) => fs::remove_file(m.path()),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Mail entry not found",
            )),
        }
    }

    /// Creates all neccessary directories if they don't exist yet. It is the library user's
    /// responsibility to call this before using `store_new`.
    pub fn create_dirs(&self) -> std::io::Result<()> {
        let mut path = self.path.clone();
        for d in &["cur", "new", "tmp"] {
            path.push(d);
            fs::create_dir_all(path.as_path())?;
            path.pop();
        }
        Ok(())
    }

    /// Stores the given message data as a new message file in the Maildir `new` folder. Does not
    /// create the neccessary directories, so if in doubt call `create_dirs` before using
    /// `store_new`.
    /// Returns the Id of the inserted message on success.
    pub fn store_new(&self, data: &[u8]) -> std::result::Result<String, MaildirError> {
        self.store(Subfolder::New, data, "")
    }

    /// Stores the given message data as a new message file in the Maildir `cur` folder, adding the
    /// given `flags` to it. The possible flags are explained e.g. at
    /// <https://cr.yp.to/proto/maildir.html> or <http://www.courier-mta.org/maildir.html>.
    /// Returns the Id of the inserted message on success.
    pub fn store_cur_with_flags(
        &self,
        data: &[u8],
        flags: &str,
    ) -> std::result::Result<String, MaildirError> {
        self.store(
            Subfolder::Cur,
            data,
            &format!(
                "{}2,{}",
                INFORMATIONAL_SUFFIX_SEPARATOR,
                Self::normalize_flags(flags)
            ),
        )
    }

    fn store(
        &self,
        subfolder: Subfolder,
        data: &[u8],
        info: &str,
    ) -> std::result::Result<String, MaildirError> {
        // try to get some uniquenes, as described at http://cr.yp.to/proto/maildir.html
        // dovecot and courier IMAP use <timestamp>.M<usec>P<pid>.<hostname> for tmp-files and then
        // move to <timestamp>.M<usec>P<pid>V<dev>I<ino>.<hostname>,S=<size_in_bytes> when moving
        // to new dir. see for example http://www.courier-mta.org/maildir.html.
        let pid = std::process::id();
        let hostname = gethostname::gethostname()
            .into_string()
            // the hostname is always ASCII in order to be a valid DNS
            // name, so into_string() will always succeed. The error case
            // here is to satisfy the compiler which doesn't know this.
            .unwrap_or_else(|_| "localhost".to_string());

        // loop when conflicting filenames occur, as described at
        // http://www.courier-mta.org/maildir.html
        // this assumes that pid and hostname don't change.
        let mut tmppath = self.path.clone();
        tmppath.push("tmp");

        let mut file;
        let mut secs;
        let mut nanos;
        let mut counter;

        loop {
            let ts = time::SystemTime::now().duration_since(time::UNIX_EPOCH)?;
            secs = ts.as_secs();
            nanos = ts.subsec_nanos();
            counter = COUNTER.fetch_add(1, Ordering::SeqCst);

            tmppath.push(format!("{secs}.#{counter:x}M{nanos}P{pid}.{hostname}"));

            match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmppath)
            {
                Ok(f) => {
                    file = f;
                    break;
                }
                Err(err) => {
                    if err.kind() != ErrorKind::AlreadyExists {
                        return Err(err.into());
                    }
                    tmppath.pop();
                }
            }
        }

        /// At this point, `file` is our new file at `tmppath`.
        /// If we leave the scope of this function prior to
        /// successfully writing the file to its final location,
        /// we need to ensure that we remove the temporary file.
        /// This struct takes care of that detail.
        struct UnlinkOnError {
            path_to_unlink: Option<PathBuf>,
        }

        impl Drop for UnlinkOnError {
            fn drop(&mut self) {
                if let Some(path) = self.path_to_unlink.take() {
                    // Best effort to remove it
                    std::fs::remove_file(path).ok();
                }
            }
        }

        // Ensure that we remove the temporary file on failure
        let mut unlink_guard = UnlinkOnError {
            path_to_unlink: Some(tmppath.clone()),
        };

        file.write_all(data)?;
        file.sync_all()?;

        let meta = file.metadata()?;
        let mut newpath = self.path.clone();
        newpath.push(match subfolder {
            Subfolder::New => "new",
            Subfolder::Cur => "cur",
        });

        #[cfg(unix)]
        let dev = meta.dev();
        #[cfg(windows)]
        let dev: u64 = 0;

        #[cfg(unix)]
        let ino = meta.ino();
        #[cfg(windows)]
        let ino: u64 = 0;

        #[cfg(unix)]
        let size = meta.size();
        #[cfg(windows)]
        let size = meta.file_size();

        let id = format!("{secs}.#{counter:x}M{nanos}P{pid}V{dev}I{ino}.{hostname},S={size}");
        newpath.push(format!("{}{}", id, info));

        std::fs::rename(&tmppath, &newpath)?;
        unlink_guard.path_to_unlink.take();
        Ok(id)
    }
}

impl From<PathBuf> for Maildir {
    fn from(p: PathBuf) -> Maildir {
        Maildir { path: p }
    }
}

impl From<String> for Maildir {
    fn from(s: String) -> Maildir {
        Maildir::from(PathBuf::from(s))
    }
}

impl<'a> From<&'a str> for Maildir {
    fn from(s: &str) -> Maildir {
        Maildir::from(PathBuf::from(s))
    }
}
