//! Symmetric exclusions: omitted entries keep their baseline and cannot turn
//! into deletions. Ancestors containing excluded data cannot be moved/deleted.
use crate::{
    filesystem,
    model::{Baselines, LocalTree, RemoteTree},
};
use anyhow::{bail, Context, Result};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::collections::HashSet;

pub fn defaults() -> Vec<String> {
    [
        "~$*",
        "*.tmp",
        "*.temp",
        "*.swp",
        "*.swo",
        "*~",
        "*.crdownload",
        "*.part",
        "*.partial",
        "*.download",
        "desktop.ini",
        "Thumbs.db",
        ".DS_Store",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

pub struct Exclusions {
    names: GlobSet,
    paths: GlobSet,
}
impl Exclusions {
    pub fn new(patterns: &[String]) -> Result<Self> {
        if patterns.len() > 128 {
            bail!("At most 128 exclusion patterns are supported");
        }
        let mut names = GlobSetBuilder::new();
        let mut paths = GlobSetBuilder::new();
        for pattern in patterns {
            if pattern.is_empty()
                || pattern.len() > 256
                || pattern.starts_with(['/', '!'])
                || pattern.contains('\\')
                || pattern.chars().any(char::is_control)
                || pattern
                    .split('/')
                    .any(|part| part.is_empty() || matches!(part, "." | ".."))
            {
                bail!("Invalid exclusion pattern: {pattern:?}; use relative glob patterns and forward slashes");
            }
            let pattern = filesystem::folded(pattern);
            let glob = GlobBuilder::new(&pattern)
                .literal_separator(true)
                .backslash_escape(false)
                .build()
                .context("Invalid exclusion glob")?;
            if pattern.contains('/') {
                paths.add(glob);
            } else {
                names.add(glob);
            }
        }
        Ok(Self {
            names: names.build()?,
            paths: paths.build()?,
        })
    }

    pub fn matches(&self, path: &str) -> bool {
        let folded = filesystem::folded(path);
        let mut end = 0;
        for name in folded.split('/') {
            end += name.len();
            if self.names.is_match(name) || self.paths.is_match(&folded[..end]) {
                return true;
            }
            end += 1;
        }
        false
    }

    pub fn scope<'a>(
        &self,
        local: &mut LocalTree,
        remote: &mut RemoteTree,
        base: &mut Baselines,
        omitted_local: impl Iterator<Item = &'a String>,
    ) -> HashSet<String> {
        let mut protected = HashSet::new();
        let mut protect = |path: &str| {
            let mut ancestor = path;
            while let Some((parent, _)) = ancestor.rsplit_once('/') {
                protected.insert(parent.to_owned());
                ancestor = parent;
            }
        };
        for path in local
            .keys()
            .chain(remote.keys())
            .chain(base.keys())
            .filter(|p| self.matches(p))
        {
            protect(path);
        }
        for path in omitted_local {
            protect(path);
        }
        local.retain(|path, _| !self.matches(path));
        remote.retain(|path, _| !self.matches(path));
        base.retain(|path, _| !self.matches(path));
        protected
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn patterns_match_names_subtrees_relative_paths_and_unicode_consistently() {
        let rules = Exclusions::new(&[
            "*.tmp".into(),
            "Cache".into(),
            "Reports/**/draft?.pdf".into(),
            "BÜRO/privat.*".into(),
        ])
        .unwrap();
        for path in [
            "a.TMP",
            "nested/a.tmp",
            "Cache/a.txt",
            "nested/cache/a.txt",
            "Reports/draft1.pdf",
            "Reports/2026/draft2.pdf",
            "Büro/privat.txt",
        ] {
            assert!(rules.matches(path), "{path}");
        }
        for path in [
            "a.tmp.txt",
            "cached.txt",
            "Reports/final.pdf",
            "Other/Reports/draft1.pdf",
            "nested/keep.txt",
        ] {
            assert!(!rules.matches(path), "{path}");
        }
        for pattern in ["", "../data", "/data", "!keep", "a\\b", "[", "a//b"] {
            assert!(Exclusions::new(&[pattern.into()]).is_err(), "{pattern}");
        }
        assert!(Exclusions::new(&vec!["*".into(); 129]).is_err());
        let defaults = Exclusions::new(&defaults()).unwrap();
        assert!(defaults.matches("Docs/~$budget.xlsx"));
        assert!(!defaults.matches("Docs/budget.xlsx"));
    }
}
