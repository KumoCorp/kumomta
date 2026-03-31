use crate::MessageConformance;
use bstr::{BStr, BString};
use std::borrow::Cow;
use std::str::Utf8Error;
use std::sync::Arc;

/// Helper for holding either an owned or borrowed string,
/// and where the slice method is aware of that borrowing,
/// allowing for efficient copying and slicing without
/// making extraneous additional copies
pub enum SharedString<'a> {
    Owned(Arc<Vec<u8>>),
    Borrowed(&'a [u8]),
    Sliced {
        other: Arc<Vec<u8>>,
        range: std::ops::Range<usize>,
    },
}

impl std::cmp::PartialEq<Self> for SharedString<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.as_bytes().eq(other.as_bytes())
    }
}

impl std::cmp::PartialEq<&str> for SharedString<'_> {
    fn eq(&self, other: &&str) -> bool {
        self.as_bytes().eq(other.as_bytes())
    }
}

impl std::fmt::Display for SharedString<'_> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s = BStr::new(self.as_bytes());
        (&s as &dyn std::fmt::Display).fmt(fmt)
    }
}

impl std::fmt::Debug for SharedString<'_> {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        let s = BStr::new(self.as_bytes());
        (&s as &dyn std::fmt::Debug).fmt(fmt)
    }
}

impl std::ops::Index<usize> for SharedString<'_> {
    type Output = u8;
    fn index(&self, index: usize) -> &u8 {
        &self.as_bytes()[index]
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
        if self.as_bytes().get(slice_range.clone()).is_none() {
            panic!("slice range {slice_range:?} is invalid for {self:?}");
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Self::Owned(s) => s,
            Self::Borrowed(s) => s,
            Self::Sliced { other, range } => other.get(range.clone()).unwrap(),
        }
    }

    pub fn to_str(&self) -> Result<&str, Utf8Error> {
        std::str::from_utf8(self.as_bytes())
    }

    pub fn to_str_lossy(&self) -> Cow<'_, str> {
        String::from_utf8_lossy(self.as_bytes())
    }

    pub fn len(&self) -> usize {
        match self {
            Self::Owned(s) => s.len(),
            Self::Borrowed(s) => s.len(),
            Self::Sliced { range, .. } => range.len(),
        }
    }

    pub fn to_owned(&'_ self) -> SharedString<'static> {
        match self {
            SharedString::Owned(s) => SharedString::Owned(Arc::clone(s)),
            SharedString::Borrowed(s) => SharedString::Owned(Arc::new(s.to_vec())),
            SharedString::Sliced { other, range } => SharedString::Sliced {
                other: other.clone(),
                range: range.clone(),
            },
        }
    }
}

impl From<BString> for SharedString<'_> {
    fn from(s: BString) -> Self {
        let v: Vec<u8> = s.into();
        v.into()
    }
}

impl From<String> for SharedString<'_> {
    fn from(s: String) -> Self {
        Self::Owned(Arc::new(s.into_bytes()))
    }
}

impl From<Vec<u8>> for SharedString<'_> {
    fn from(s: Vec<u8>) -> Self {
        Self::Owned(Arc::new(s))
    }
}

impl<'a> From<&'a str> for SharedString<'a> {
    fn from(s: &'a str) -> Self {
        Self::Borrowed(s.as_bytes())
    }
}

impl<'a> From<&'a [u8]> for SharedString<'a> {
    fn from(s: &'a [u8]) -> Self {
        Self::Borrowed(s)
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
            SharedString::Owned(Arc::new(self.into_bytes())),
            MessageConformance::default(),
        )
    }
}

impl<'a> IntoSharedString<'a> for &'a str {
    fn into_shared_string(self) -> (SharedString<'a>, MessageConformance) {
        (
            SharedString::Borrowed(self.as_bytes()),
            MessageConformance::default(),
        )
    }
}

impl<'a> IntoSharedString<'a> for &'a [u8] {
    fn into_shared_string(self) -> (SharedString<'a>, MessageConformance) {
        match std::str::from_utf8(self) {
            Ok(s) => (
                SharedString::Borrowed(s.as_bytes()),
                MessageConformance::default(),
            ),
            Err(_) => (
                SharedString::Borrowed(self),
                MessageConformance::NEEDS_TRANSFER_ENCODING,
            ),
        }
    }
}
