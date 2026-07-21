use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rust_rule_engine::GRLParser;
use serde_json::{Value, json};

pub struct RulesRepository {
    repository_dir: PathBuf,
    rules_path: PathBuf,
    directories: Vec<String>,
}

impl RulesRepository {
    pub fn open(
        repository_dir: &Path,
        rules_path: PathBuf,
        directories: Vec<String>,
    ) -> Result<Self, String> {
        if rules_path.is_absolute() {
            return Err("repository_rules_path must be relative".to_string());
        }
        gix::open(repository_dir)
            .map_err(|e| format!("open rules repository {}: {e}", repository_dir.display()))?;
        Ok(Self {
            repository_dir: repository_dir.to_path_buf(),
            rules_path,
            directories,
        })
    }

    pub fn head(&self) -> Result<Value, String> {
        let repo = self.repo()?;
        let head = repo.head().map_err(|e| e.to_string())?;
        let branch = head
            .referent_name()
            .map(|name| name.shorten().to_string())
            .unwrap_or_else(|| "detached".to_string());
        let sha = repo
            .head_id()
            .map_err(|e| e.to_string())?
            .to_hex()
            .to_string();
        Ok(json!({ "branch": branch, "sha": sha }))
    }

    pub fn branches(&self) -> Result<Value, String> {
        let repo = self.repo()?;
        let platform = repo.references().map_err(|e| e.to_string())?;
        let branches = platform
            .local_branches()
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .map(|reference| reference.name().shorten().to_string())
            .collect::<Vec<_>>();
        Ok(json!(branches))
    }

    pub fn load_at(&self, revision: &str) -> Result<String, String> {
        let repo = self.repo()?;
        let mut rules = String::new();
        for directory in &self.directories {
            let mut files = self.read_grl_files(&repo, revision, directory)?;
            files.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, content) in files {
                GRLParser::parse_rules(&content)
                    .map_err(|error| format!("parse {revision}:{name}: {error}"))?;
                rules.push_str(&content);
                rules.push('\n');
            }
        }
        Ok(rules)
    }

    pub fn diff(&self, a: &str, b: &str) -> Result<Value, String> {
        let repo = self.repo()?;
        let tree_a = Self::tree_of(&repo, a)?;
        let tree_b = Self::tree_of(&repo, b)?;
        let mut recorder = gix::diff::tree::Recorder::default();
        gix::diff::tree(
            gix::objs::TreeRefIter::from_bytes(&tree_a.data, tree_a.id.kind()),
            gix::objs::TreeRefIter::from_bytes(&tree_b.data, tree_b.id.kind()),
            &mut gix::diff::tree::State::default(),
            &repo,
            &mut recorder,
        )
        .map_err(|e| format!("diff `{a}` to `{b}`: {e:?}"))?;
        let changes = recorder
            .records
            .into_iter()
            .filter_map(|change| {
                use gix::diff::tree::recorder::Change;
                let (path, status) = match change {
                    Change::Addition { path, .. } => (path.to_string(), "added"),
                    Change::Deletion { path, .. } => (path.to_string(), "removed"),
                    Change::Modification { path, .. } => (path.to_string(), "modified"),
                };
                self.is_rule_path(&path).then(|| {
                    json!({
                        "path": self.display_rule_path(&path),
                        "status": status,
                    })
                })
            })
            .collect::<Vec<_>>();
        Ok(json!({ "a": a, "b": b, "changes": changes }))
    }

    pub fn compare(&self, a: &str, b: &str) -> Result<Value, String> {
        let rules_a = self.load_at(a)?;
        let rules_b = self.load_at(b)?;
        compare_rules(&rules_a, &rules_b, a, b)
    }

    pub fn promote(&self, from: &str, to: &str) -> Result<Value, String> {
        let repo = self.repo()?;
        let from_id = repo
            .rev_parse_single(from)
            .map_err(|e| format!("rev `{from}`: {e:?}"))?
            .detach();
        let to_id = repo
            .rev_parse_single(to)
            .map_err(|e| format!("rev `{to}`: {e:?}"))?
            .detach();
        let merge_base = repo
            .merge_base(from_id, to_id)
            .map_err(|e| e.to_string())?
            .detach();
        if merge_base != to_id {
            return Err(format!(
                "`{from}` is not a fast-forward of `{to}`; rebase the proposal first"
            ));
        }
        repo.reference(
            format!("refs/heads/{to}"),
            from_id,
            gix::refs::transaction::PreviousValue::Any,
            "promote rules",
        )
        .map_err(|e| format!("update {to}: {e:?}"))?;
        Ok(json!({ "from": from, "to": to, "sha": from_id.to_hex().to_string() }))
    }

    fn read_grl_files(
        &self,
        repo: &gix::Repository,
        revision: &str,
        directory: &str,
    ) -> Result<Vec<(String, String)>, String> {
        let tree = Self::tree_of(repo, revision)?;
        let path = self.rules_path.join(directory);
        let Some(entry) = tree
            .lookup_entry_by_path(&path)
            .map_err(|e| e.to_string())?
        else {
            return Ok(Vec::new());
        };
        let directory_tree = repo
            .find_object(entry.object_id())
            .map_err(|e| e.to_string())?
            .into_tree();
        let mut files = Vec::new();
        for entry in directory_tree.iter() {
            let entry = entry.map_err(|e| e.to_string())?;
            let filename = entry.filename();
            if filename.ends_with(b".grl") {
                let blob = repo
                    .find_object(entry.object_id())
                    .map_err(|e| e.to_string())?
                    .into_blob();
                let content = String::from_utf8(blob.data.to_vec()).map_err(|e| e.to_string())?;
                files.push((String::from_utf8_lossy(filename).into_owned(), content));
            }
        }
        Ok(files)
    }

    fn tree_of<'repo>(
        repo: &'repo gix::Repository,
        revision: &str,
    ) -> Result<gix::Tree<'repo>, String> {
        repo.rev_parse_single(revision)
            .map_err(|e| format!("rev `{revision}`: {e:?}"))?
            .object()
            .map_err(|e| format!("load revision `{revision}` object: {e:?}"))?
            .peel_to_tree()
            .map_err(|e| format!("peel revision `{revision}` to tree: {e:?}"))
    }

    fn repo(&self) -> Result<gix::Repository, String> {
        gix::open(&self.repository_dir).map_err(|e| {
            format!(
                "open rules repository {}: {e}",
                self.repository_dir.display()
            )
        })
    }

    fn is_rule_path(&self, path: &str) -> bool {
        let path = Path::new(path);
        path.extension().is_some_and(|extension| extension == "grl")
            && self
                .directories
                .iter()
                .any(|directory| path.starts_with(self.rules_path.join(directory)))
    }

    fn display_rule_path(&self, path: &str) -> String {
        Path::new(path)
            .strip_prefix(&self.rules_path)
            .unwrap_or_else(|_| Path::new(path))
            .to_string_lossy()
            .into_owned()
    }
}

fn compare_rules(rules_a: &str, rules_b: &str, a: &str, b: &str) -> Result<Value, String> {
    let map_a = rules_by_name(rules_a)?;
    let map_b = rules_by_name(rules_b)?;
    let only_a = map_a
        .keys()
        .filter(|name| !map_b.contains_key(*name))
        .collect::<Vec<_>>();
    let only_b = map_b
        .keys()
        .filter(|name| !map_a.contains_key(*name))
        .collect::<Vec<_>>();
    let changed = map_a
        .iter()
        .filter_map(|(name, representation_a)| {
            map_b
                .get(name)
                .filter(|representation_b| *representation_b != representation_a)
                .map(|_| name)
        })
        .collect::<Vec<_>>();
    let unchanged = map_a
        .iter()
        .filter(|(name, representation_a)| {
            map_b
                .get(*name)
                .is_some_and(|representation_b| representation_b == *representation_a)
        })
        .count();
    Ok(json!({
        "a": a,
        "b": b,
        "only_a": only_a,
        "only_b": only_b,
        "changed": changed,
        "unchanged": unchanged,
    }))
}

fn rules_by_name(grl: &str) -> Result<BTreeMap<String, String>, String> {
    Ok(GRLParser::parse_rules(grl)
        .map_err(|error| error.to_string())?
        .iter()
        .map(|rule| {
            (
                rule.name.clone(),
                format!("{:?}\n{:?}", rule.conditions, rule.actions),
            )
        })
        .collect())
}
