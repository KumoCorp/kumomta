use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpoolId(Uuid);

impl std::fmt::Display for SpoolId {
    fn fmt(&self, fmt: &mut std::fmt::Formatter) -> std::fmt::Result {
        self.0.simple().fmt(fmt)
    }
}

impl SpoolId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }

    pub fn compute_path(&self, in_dir: &Path) -> PathBuf {
        let (a, b, c, [d, e, f, g, h, i, j, k]) = self.0.as_fields();
        let name = format!(
            "{a:08x}/{b:04x}/{c:04x}/{d:02x}{e:02x}/{f:02x}{g:02x}{h:02x}{i:02x}{j:02x}{k:02x}"
        );
        in_dir.join(name)
    }

    pub fn from_path(mut path: &Path) -> Option<Self> {
        let mut components = vec![];

        for _ in 0..5 {
            components.push(path.file_name()?.to_str()?);
            path = path.parent()?;
        }

        components.reverse();
        Some(Self(Uuid::try_parse(&components.join("-")).ok()?))
    }
}
