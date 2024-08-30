pub trait MetricLabel {
    fn label_names() -> &'static [&'static str];
    fn emit_text_value(&self, target: &mut String, value: &str);
    fn emit_json_value(&self, target: &mut String, value: &str);
}

/// `macro_rules!` implementation of `count_tts`.
/// Source: https://github.com/camsteffen/count-tts
#[macro_export]
macro_rules! count_tts {
    () => (0);
    ($one:tt) => (1);
    ($($a:tt $b:tt)+) => ($crate::count_tts!($($a)+) << 1);
    ($odd:tt $($a:tt $b:tt)+) => ($crate::count_tts!($($a)+) << 1 | 1);
}

/// Used to declare a label key struct suitable for use in registering
/// counters in the counter registry.
///
/// Usage looks like:
///
/// ```ignore
/// label_key! {
///    pub struct LabelKey {
///       pub label1: String,
///    }
/// }
/// ```
///
/// Always include the trailing comma after each struct field!
///
/// The macro will also generate `BorrowedLabelKey` and `LabelKeyTrait`
/// types.  The `LabelKeyTrait` is implemented for both `LabelKey` and
/// `BorrowedLabelKey` and provides a `key()` method that will return a
/// `BorrowedLabelKey` for either.
///
/// The `BorrowedLabelKey` offers a `to_owned()` method to return a
/// `LabelKey`, and a `label_pairs()` method to return a fixed size
/// array representation consisting of the key and value pairs:
///
/// ```ignore
/// assert_eq!(BorrowedLabelKey { label1: "hello"}.label_pairs(),
///   [("label1", "hello")]
/// )
/// ```
#[macro_export]
macro_rules! label_key {
    (pub struct $name:ident {
        $(
            pub $fieldname:ident: String,
        )*
    }
    ) => {
        $crate::paste::paste!{
            #[derive(Clone, Hash, Eq, PartialEq)]
            pub struct $name {
                $(
                    pub $fieldname: String,
                )*
            }

            pub trait [<$name Trait>] {
                fn key<'k>(&'k self) -> [<Borrowed $name>]<'k>;
            }

            impl [<$name Trait>] for $name {
                fn key<'k>(&'k self) -> [<Borrowed $name>]<'k> {
                    [<Borrowed $name>] {
                        $(
                            $fieldname: self.$fieldname.as_str(),
                        )*
                    }
                }
            }

            // <https://github.com/sunshowers-code/borrow-complex-key-example/blob/main/src/lib.rs>
            // has a detailed explanation of this stuff.
            #[derive(Clone, Copy, Hash, Eq, PartialEq)]
            pub struct [<Borrowed $name>]<'a> {
                $(
                    pub $fieldname: &'a str,
                )*
            }

            impl<'a> [<Borrowed $name>] <'a> {
                #[allow(unused)]
                pub fn to_owned(&self) -> $name {
                    $name {
                        $(
                            $fieldname: self.$fieldname.to_string(),
                        )*
                    }
                }

                #[allow(unused)]
                pub fn label_pairs(&self) -> [(&str,&str); $crate::count_tts!($($fieldname)*)] {
                    [
                        $(
                            (stringify!($fieldname), self.$fieldname),
                        )*
                    ]
                }
            }

            impl<'a> From<&'a [<Borrowed $name>]<'a>> for $name {
                fn from(value: &'a [<Borrowed $name>]<'_>) -> $name {
                    value.to_owned()
                }
            }

            impl<'a> From<&'a dyn [<$name Trait>]> for $name {
                fn from(value: &'a (dyn [<$name Trait>] + 'a)) -> $name {
                    value.key().to_owned()
                }
            }

            impl<'a> [<$name Trait>] for [<Borrowed $name>] <'a> {
                fn key<'k>(&'k self) -> [<Borrowed $name>]<'k> {
                    *self
                }
            }

            impl<'a> ::std::borrow::Borrow<dyn [<$name Trait>] + 'a> for $name {
                fn borrow(&self) -> &(dyn [<$name Trait>] + 'a) {
                    self
                }
            }

            impl<'a> PartialEq for (dyn [<$name Trait>] + 'a) {
                fn eq(&self, other: &Self) -> bool {
                    self.key().eq(&other.key())
                }
            }

            impl<'a> Eq for (dyn [<$name Trait>] + 'a) {}
            impl<'a> ::std::hash::Hash for (dyn [<$name Trait>] + 'a) {
                fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
                    self.key().hash(state)
                }
            }

            impl $crate::labels::MetricLabel for $name {
                fn label_names() -> &'static [&'static str] {
                    const LABEL_NAMES: &'static [&'static str] = &[
                        $(
                            stringify!($fieldname),
                        )*
                    ];

                    LABEL_NAMES
                }

                fn emit_text_value(&self, target: &mut String, value: &str) {
                    let key = self.key();
                    let pairs = key.label_pairs();
                    target.push('{');
                    for (i, (key, value)) in pairs.iter().enumerate() {
                        if i > 0 {
                            target.push_str(", ");
                        }
                        target.push_str(key);
                        target.push_str("=\"");
                        target.push_str(value);
                        target.push_str("\"");
                    }
                    target.push_str("} ");
                    target.push_str(value);
                }

                fn emit_json_value(&self, target: &mut String, value: &str) {
                    let key = self.key();
                    let pairs = key.label_pairs();

                    if pairs.len() == 1 {
                        target.push('"');
                        target.push_str(pairs[0].1);
                        target.push_str("\":");
                        target.push_str(value);
                    } else {
                        target.push_str("{");
                        for (key, value) in pairs.iter() {
                            target.push_str("\"");
                            target.push_str(key);
                            target.push_str("\":\"");
                            target.push_str(value);
                            target.push_str("\",");
                        }

                        target.push_str("\"@\":");
                        target.push_str(value);
                        target.push_str("}");
                    }
                }
            }
        }
    };
}

#[cfg(test)]
mod test {
    #[test]
    fn test_label_macro() {
        label_key! {
            pub struct MyLabel {
                pub myname: String,
            }
        }

        assert_eq!(
            BorrowedMyLabel { myname: "hello" }.label_pairs(),
            [("myname", "hello")]
        );
    }

    #[test]
    fn test_labels_macro() {
        label_key! {
            pub struct MyLabels {
                pub myname: String,
                pub second_name: String,
            }
        }

        assert_eq!(
            BorrowedMyLabels {
                myname: "hello",
                second_name: "there"
            }
            .label_pairs(),
            [("myname", "hello"), ("second_name", "there")]
        );
    }
}
