use anyhow::{bail, ensure, Context, Result};

pub fn from_directory(path: &str) -> Result<impl Iterator<Item = Result<PackageDesc>>> {
    let iter = std::fs::read_dir(path).context("read_dir")?;
    Ok(iter
        .map(|entry| {
            let entry = entry.context("entry")?;
            if !entry.file_type().context("file_type")?.is_dir() {
                return Ok(None);
            }
            let mut path = entry.path();
            path.push("desc");
            let contents = std::fs::read_to_string(&path).context(format!("read {:?}", path))?;
            let desc =
                PackageDesc::parse(contents.as_str()).context(format!("parse {:?}", path))?;
            Ok(Some(desc))
        })
        .filter_map(Result::transpose))
}

#[derive(Debug, Default)]
pub struct PackageDesc {
    pub name: String,
    pub version: String,
    pub description: String,
    pub url: String,
    pub reason: Reason,
    pub size: Option<u64>,
    pub dependencies: Vec<String>,
    pub optional_dependencies: Vec<OptionalDependency>,
}

#[derive(Clone, Copy, Debug, Default)]
pub enum Reason {
    #[default]
    Explicit,
    Dependency,
}

#[derive(Debug, Default)]
pub struct OptionalDependency {
    pub name: String,
    pub description: Option<String>,
}

impl OptionalDependency {
    // without ending newline
    fn parse(line: &str) -> Self {
        let mut split = line.split(": ").map(ToString::to_string);
        // Unwrap because split always has at least one item.
        let name = split.next().unwrap();
        let description = split.next();
        Self { name, description }
    }
}

#[test]
fn parse_optional_dependency_without_reason() {
    let a = OptionalDependency::parse("name");
    assert_eq!(a.name, "name");
    assert_eq!(a.description, None);
}

#[test]
fn parse_optional_dependency_with_reason() {
    let a = OptionalDependency::parse("name: reason");
    assert_eq!(a.name, "name");
    assert_eq!(a.description, Some("reason".to_string()));
}

impl PackageDesc {
    fn parse(s: &str) -> Result<Self> {
        let mut self_ = Self::default();
        for section in s.split_terminator("\n\n") {
            let mut lines = section.split_terminator('\n');
            let name = lines.next().context("section has no name")?;
            let first_body = lines.next().context("section has no content")?;
            match name {
                "%NAME%" => {
                    self_.name = first_body.to_string();
                }
                "%VERSION%" => {
                    self_.version = first_body.to_string();
                }
                "%DESC%" => {
                    self_.description = first_body.to_string();
                }
                "%URL%" => {
                    self_.url = first_body.to_string();
                }
                "%REASON%" => {
                    self_.reason = match first_body {
                        "1" => Reason::Dependency,
                        _ => bail!("unexpected reason {first_body:?}"),
                    }
                }
                "%SIZE%" => {
                    self_.size = Some(
                        first_body
                            .parse()
                            .context(format!("parse size {first_body:?}"))?,
                    );
                }
                "%DEPENDS%" => {
                    self_.dependencies.push(first_body.to_string());
                    self_.dependencies.extend(lines.map(ToString::to_string));
                }
                "%OPTDEPENDS%" => {
                    self_
                        .optional_dependencies
                        .push(OptionalDependency::parse(first_body));
                    self_
                        .optional_dependencies
                        .extend(lines.map(OptionalDependency::parse));
                }
                _ => (),
            }
        }
        ensure!(!self_.name.is_empty());
        ensure!(!self_.version.is_empty());
        ensure!(!self_.description.is_empty());
        ensure!(!self_.url.is_empty());
        Ok(self_)
    }
}
