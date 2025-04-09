use crate::MessageConformance;
use std::sync::Arc;

/// Helper for holding either an owned or borrowed string,
/// and where the slice method is aware of that borrowing,
/// allowing for efficient copying and slicing without
/// making extraneous additional copies
pub enum SharedString<'a> {
    Owned(Arc<String>),
    Borrowed(&'a str),
    Sliced {
        other: Arc<String>,
        range: std::ops::Range<usize>,
    },
}

impl std::cmp::PartialEq<Self> for SharedString<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_str().eq(other.as_str())
    }
}

impl std::cmp::PartialEq<&str> for SharedString<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.as_str().eq(*other)
    }
}

impl std::fmt::Display for SharedString<'_> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let str = self.as_str();
        fmt.write_str(str)
    }
}

impl std::fmt::Debug for SharedString<'_> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let str = self.as_str();
        write!(fmt, "{str:?}")
    }
}

impl std::ops::Deref for SharedString<'_> {
    type Target = str;
    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl std::ops::Index<usize> for SharedString<'_> {
    type Output = u8;
    fn index(&self, index: usize) -> &u8 {
        &self.as_str().as_bytes()[index]
    }
}

impl Clone for SharedString<'_> {
    fn clone(&self) -> Self {
        match self {
            Self::Owned(s) => Self::Sliced {
                other: Arc::clone(s),
                range: 0..s.len(),
            },
            Self::Borrowed(s) => Self::Borrowed(s),
            Self::Sliced { other, range } => Self::Sliced {
                other: Arc::clone(other),
                range: range.clone(),
            },
        }
    }
}

impl SharedString<'_> {
    pub fn slice(&self, slice_range: std::ops::Range<usize>) -> Self {
        self.assert_slice(slice_range.clone());
        match self {
            Self::Owned(s) => Self::Sliced {
                other: Arc::clone(s),
                range: slice_range,
            },
            Self::Borrowed(s) => Self::Borrowed(s.get(slice_range).unwrap()),
            Self::Sliced { other, range } => {
                let len = slice_range.end - slice_range.start;
                Self::Sliced {
                    other: Arc::clone(other),
                    range: range.start + slice_range.start..range.start + slice_range.start + len,
                }
            }
        }
    }

    fn assert_slice(&self, slice_range: std::ops::Range<usize>) {
        if self.as_str().get(slice_range.clone()).is_none() {
            panic!("slice range {slice_range:?} is invalid for {self:?}");
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Owned(s) => s.as_str(),
            Self::Borrowed(s) => s,
            Self::Sliced { other, range } => other.as_str().get(range.clone()).unwrap(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Owned(s) => s.len(),
            Self::Borrowed(s) => s.len(),
            Self::Sliced { range, .. } => range.len(),
        }
    }
}

impl From<String> for SharedString<'_> {
    fn from(s: String) -> Self {
        Self::Owned(Arc::new(s))
    }
}

impl<'a> From<&'a str> for SharedString<'a> {
    fn from(s: &'a str) -> Self {
        Self::Borrowed(s)
    }
}

impl<'a> TryFrom<&'a [u8]> for SharedString<'a> {
    type Error = std::str::Utf8Error;
    fn try_from(s: &'a [u8]) -> Result<Self, Self::Error> {
        let s = std::str::from_utf8(s)?;
        Ok(Self::Borrowed(s))
    }
}

pub trait IntoSharedString<'a> {
    fn into_shared_string(self) -> (SharedString<'a>, MessageConformance);
}

impl<'a> IntoSharedString<'a> for SharedString<'a> {
    fn into_shared_string(self) -> (SharedString<'a>, MessageConformance) {
        (self, MessageConformance::default())
    }
}

impl<'a> IntoSharedString<'a> for String {
    fn into_shared_string(self) -> (SharedString<'a>, MessageConformance) {
        (
            SharedString::Owned(Arc::new(self)),
            MessageConformance::default(),
        )
    }
}

impl<'a> IntoSharedString<'a> for &'a str {
    fn into_shared_string(self) -> (SharedString<'a>, MessageConformance) {
        (SharedString::Borrowed(self), MessageConformance::default())
    }
}

impl<'a> IntoSharedString<'a> for &'a [u8] {
    fn into_shared_string(self) -> (SharedString<'a>, MessageConformance) {
        match std::str::from_utf8(self) {
            Ok(s) => (SharedString::Borrowed(s), MessageConformance::default()),
            Err(_) => (
                SharedString::Owned(Arc::new(String::from_utf8_lossy(self).to_string())),
                MessageConformance::NEEDS_TRANSFER_ENCODING,
            ),
        }
    }
}
