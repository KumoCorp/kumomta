use config::{any_err, get_or_create_sub_module};
use fancy_regex::{Matches, Regex};
use mlua::{Lua, UserData, UserDataMethods};

struct RegexWrap(Regex);

impl UserData for RegexWrap {
    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("captures", |lua, this, haystack: String| {
            match this.0.captures(&haystack).map_err(any_err)? {
                Some(c) => {
                    let result = lua.create_table()?;

                    let names = this.0.capture_names();
                    for ((idx, cap), name) in c.iter().enumerate().zip(names) {
                        if let Some(cap) = cap {
                            let s = cap.as_str();
                            result.set(idx, s.to_string())?;
                            if let Some(name) = name {
                                result.set(name, s.to_string())?;
                            }
                        }
                    }

                    Ok(Some(result))
                }
                None => Ok(None),
            }
        });

        methods.add_method("is_match", |_, this, haystack: String| {
            Ok(this.0.is_match(&haystack).map_err(any_err)?)
        });

        methods.add_method("find", |_, this, haystack: String| {
            Ok(this
                .0
                .find(&haystack)
                .map_err(any_err)?
                .map(|m| m.as_str().to_string()))
        });

        methods.add_method("find_all", |_, this, haystack: String| {
            let mut result = vec![];

            for m in this.0.find_iter(&haystack) {
                let s = m.map_err(any_err)?;
                result.push(s.as_str().to_string());
            }
            Ok(result)
        });

        methods.add_method("replace", |_, this, (haystack, rep): (String, String)| {
            Ok(this.0.replace(&haystack, &rep).to_string())
        });

        methods.add_method(
            "replace_all",
            |_, this, (haystack, rep): (String, String)| {
                Ok(this
                    .0
                    .try_replacen(&haystack, 0, &rep)
                    .map_err(any_err)?
                    .to_string())
            },
        );

        methods.add_method(
            "replacen",
            |_, this, (haystack, limit, rep): (String, usize, String)| {
                Ok(this
                    .0
                    .try_replacen(&haystack, limit, &rep)
                    .map_err(any_err)?
                    .to_string())
            },
        );

        methods.add_method("split", |_, this, haystack: String| {
            Ok(split_into_vec(&this.0, &haystack).map_err(any_err)?)
        });
    }
}

pub fn register(lua: &Lua) -> anyhow::Result<()> {
    let regex_mod = get_or_create_sub_module(lua, "regex")?;

    regex_mod.set(
        "compile",
        lua.create_function(move |_, pattern: String| {
            let re = Regex::new(&pattern).map_err(any_err)?;
            Ok(RegexWrap(re))
        })?,
    )?;

    regex_mod.set(
        "escape",
        lua.create_function(move |_, pattern: String| Ok(regex::escape(&pattern)))?,
    )?;

    Ok(())
}

struct Split<'r, 'h> {
    finder: Matches<'r, 'h>,
    last: usize,
    haystack: &'h str,
}

impl<'r, 'h> Split<'r, 'h> {
    fn split(re: &'r Regex, haystack: &'h str) -> Self {
        Self {
            finder: re.find_iter(haystack),
            last: 0,
            haystack,
        }
    }
}

impl<'r, 'h> Iterator for Split<'r, 'h> {
    type Item = Result<&'h str, fancy_regex::Error>;

    fn next(&mut self) -> Option<Result<&'h str, fancy_regex::Error>> {
        match self.finder.next() {
            None => {
                let len = self.haystack.len();
                if self.last > len {
                    None
                } else {
                    let span = &self.haystack[self.last..len];
                    self.last = len + 1; // Next call will return None
                    Some(Ok(span))
                }
            }
            Some(Ok(m)) => {
                let span = &self.haystack[self.last..m.start()];
                self.last = m.end();
                Some(Ok(span))
            }
            Some(Err(e)) => Some(Err(e)),
        }
    }
}

fn split_into_vec(re: &Regex, haystack: &str) -> Result<Vec<String>, fancy_regex::Error> {
    let mut result = vec![];
    for m in Split::split(re, haystack) {
        let m = m?;
        result.push(m.to_string());
    }
    Ok(result)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn fancy_split() {
        let re = Regex::new("[ \t]+").unwrap();
        let hay = "a b \t  c\td    e";
        let fields = split_into_vec(&re, hay).unwrap();
        assert_eq!(fields, vec!["a", "b", "c", "d", "e"]);
    }
}
