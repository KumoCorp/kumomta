//! A registry of where live [`LogWriter`](crate::LogWriter)s are currently
//! producing segments.

use camino::Utf8PathBuf;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};

static WRITER_LOCATIONS: LazyLock<Mutex<HashMap<WriterLocation, usize>>> =
    LazyLock::new(Default::default);

/// The location of the segments produced by a writer.
#[derive(Hash, PartialEq, Eq, Debug, Clone)]
pub struct WriterLocation {
    /// The segment output directory.
    pub directory: Utf8PathBuf,
    /// Glob that identifies segments produced by this writer among unrelated
    /// files and segments from other writers in the same directory.
    pub pattern: String,
}

/// An entry in the registry for a writer, held for the lifetime of the writer.
/// While held, the location of the writer is active. Dropping it releases the
/// entry, and the location is removed once the last writer to it is dropped.
pub struct WriterRegistration {
    location: WriterLocation,
}

impl WriterRegistration {
    /// Record that a writer is producing segments (whose names end
    /// with `suffix`) in `directory`.
    pub fn new(directory: Utf8PathBuf, suffix: Option<&str>) -> Self {
        let location = WriterLocation {
            directory,
            pattern: pattern_for_suffix(suffix),
        };
        *WRITER_LOCATIONS
            .lock()
            .expect("jsonl writer registry mutex poisoned")
            .entry(location.clone())
            .or_insert(0) += 1;
        Self { location }
    }
}

impl Drop for WriterRegistration {
    fn drop(&mut self) {
        let mut locations = WRITER_LOCATIONS
            .lock()
            .expect("jsonl writer registry mutex poisoned");
        if let Some(count) = locations.get_mut(&self.location) {
            *count -= 1;
            if *count == 0 {
                locations.remove(&self.location);
            }
        }
    }
}

/// Returns the distinct locations where live writers are currently producing
/// segments.
pub fn active_writer_locations() -> Vec<WriterLocation> {
    WRITER_LOCATIONS
        .lock()
        .expect("jsonl writer registry mutex poisoned")
        .keys()
        .cloned()
        .collect()
}

/// Build the glob matching the segments written with the given `suffix`.
/// Returns the bare `*` when a suffix isn't provided.
fn pattern_for_suffix(suffix: Option<&str>) -> String {
    const GLOB_META: &[char] = &['\\', '*', '?', '[', ']', '{', '}'];
    match suffix {
        None => "*".to_string(),
        Some(suffix) => {
            let mut pattern = String::with_capacity(1 + suffix.len());
            pattern.push('*');
            for c in suffix.chars() {
                if GLOB_META.contains(&c) {
                    pattern.push('\\');
                }
                pattern.push(c);
            }
            pattern
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn suffix_to_pattern() {
        k9::assert_equal!(pattern_for_suffix(None), "*");
        k9::assert_equal!(pattern_for_suffix(Some(".log")), "*.log");
        // Glob metacharacters in the suffix become literals to match the real
        // segment names rather than acting as wildcards.
        k9::assert_equal!(pattern_for_suffix(Some("-v1[a]*")), "*-v1\\[a\\]\\*");
    }
}
