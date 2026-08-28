use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectProfile {
    pub kind: ProjectKind,
    pub evidence: Vec<String>,
    pub verification_command: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectKind {
    Rust,
    PythonPytest,
    PythonUnittest,
    Node,
    Maven,
    Gradle,
    Go,
    DotNet,
    Unknown,
}

impl ProjectKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Rust => "Rust",
            Self::PythonPytest => "Python/pytest",
            Self::PythonUnittest => "Python/unittest",
            Self::Node => "Node.js",
            Self::Maven => "Java/Maven",
            Self::Gradle => "Java/Gradle",
            Self::Go => "Go",
            Self::DotNet => ".NET",
            Self::Unknown => "Unknown",
        }
    }
}

impl ProjectProfile {
    pub fn detect(workspace: &Path) -> Self {
        if workspace.join("Cargo.toml").is_file() {
            return Self::known(ProjectKind::Rust, "Cargo.toml", "cargo test");
        }

        if workspace.join("pyproject.toml").is_file() {
            return Self::known(
                ProjectKind::PythonPytest,
                "pyproject.toml",
                "python -m pytest",
            );
        }
        if let Some(evidence) = ["pytest.ini", "tox.ini"]
            .into_iter()
            .find(|name| workspace.join(name).is_file())
        {
            return Self::known(ProjectKind::PythonPytest, evidence, "python -m pytest");
        }
        if requirements_use_pytest(workspace) {
            return Self::known(
                ProjectKind::PythonPytest,
                "requirements.txt (pytest)",
                "python -m pytest",
            );
        }

        if workspace.join("package.json").is_file() {
            return detect_node(workspace);
        }
        if workspace.join("pom.xml").is_file() {
            let command = if workspace.join("mvnw.cmd").is_file() {
                r".\mvnw.cmd test"
            } else if workspace.join("mvnw").is_file() {
                "./mvnw test"
            } else {
                "mvn test"
            };
            return Self::known(ProjectKind::Maven, "pom.xml", command);
        }
        if workspace.join("build.gradle").is_file() || workspace.join("build.gradle.kts").is_file()
        {
            let evidence = if workspace.join("build.gradle.kts").is_file() {
                "build.gradle.kts"
            } else {
                "build.gradle"
            };
            let command = if workspace.join("gradlew.bat").is_file() {
                r".\gradlew.bat test"
            } else if workspace.join("gradlew").is_file() {
                "./gradlew test"
            } else {
                "gradle test"
            };
            return Self::known(ProjectKind::Gradle, evidence, command);
        }
        if workspace.join("go.mod").is_file() {
            return Self::known(ProjectKind::Go, "go.mod", "go test ./...");
        }
        if let Some(evidence) = dotnet_evidence(workspace) {
            return Self::known(ProjectKind::DotNet, &evidence, "dotnet test");
        }
        if has_python_unittest_layout(workspace) {
            return Self::known(
                ProjectKind::PythonUnittest,
                "tests/*.py",
                "python -m unittest discover -s tests -v",
            );
        }

        Self {
            kind: ProjectKind::Unknown,
            evidence: Vec::new(),
            verification_command: None,
        }
    }

    pub fn prompt_hint(&self) -> String {
        match &self.verification_command {
            Some(command) => format!(
                "Detected project: {} (evidence: {}). Runtime-selected verification command: `{command}`.",
                self.kind.label(),
                self.evidence.join(", ")
            ),
            None => format!(
                "Detected project: {}. No deterministic verification command was found; inspect project configuration and choose a real test, build, lint, or program command.",
                self.kind.label()
            ),
        }
    }

    fn known(kind: ProjectKind, evidence: &str, command: &str) -> Self {
        Self {
            kind,
            evidence: vec![evidence.to_owned()],
            verification_command: Some(command.to_owned()),
        }
    }
}

fn detect_node(workspace: &Path) -> ProjectProfile {
    let scripts = fs::read_to_string(workspace.join("package.json"))
        .ok()
        .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
        .and_then(|value| value.get("scripts").cloned())
        .and_then(|value| value.as_object().cloned());
    let command = scripts.as_ref().and_then(|scripts| {
        if scripts
            .get("test")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|script| !script.contains("no test specified"))
        {
            Some("npm test")
        } else if scripts.contains_key("build") {
            Some("npm run build")
        } else if scripts.contains_key("lint") {
            Some("npm run lint")
        } else {
            None
        }
    });
    ProjectProfile {
        kind: ProjectKind::Node,
        evidence: vec!["package.json".to_owned()],
        verification_command: command.map(str::to_owned),
    }
}

fn requirements_use_pytest(workspace: &Path) -> bool {
    fs::read_to_string(workspace.join("requirements.txt"))
        .ok()
        .is_some_and(|content| {
            content.lines().any(|line| {
                line.trim()
                    .to_ascii_lowercase()
                    .strip_prefix("pytest")
                    .is_some_and(|rest| {
                        rest.is_empty()
                            || rest.starts_with(['=', '<', '>', '~', '!', '[', ';', ' '])
                    })
            })
        })
}

fn has_python_unittest_layout(workspace: &Path) -> bool {
    let tests = workspace.join("tests");
    if !tests.is_dir() {
        return false;
    }
    python_files(&tests, 2)
        .into_iter()
        .any(|path| is_test_python_file(&path))
}

fn python_files(root: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut pending = vec![(root.to_path_buf(), 0_usize)];
    let mut files = Vec::new();
    while let Some((directory, depth)) = pending.pop() {
        let Ok(entries) = fs::read_dir(directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() && path.extension().is_some_and(|extension| extension == "py") {
                files.push(path);
            } else if depth < max_depth && path.is_dir() && !path.is_symlink() {
                pending.push((path, depth + 1));
            }
        }
    }
    files
}

fn is_test_python_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with("test") && name.ends_with(".py"))
}

fn dotnet_evidence(workspace: &Path) -> Option<String> {
    let entries = fs::read_dir(workspace).ok()?;
    entries.flatten().find_map(|entry| {
        let path = entry.path();
        let extension = path.extension()?.to_str()?;
        matches!(extension, "sln" | "csproj").then(|| entry.file_name().to_string_lossy().into())
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::tempdir;

    use super::{ProjectKind, ProjectProfile};

    #[test]
    fn detects_rust_project() {
        let workspace = tempdir().expect("workspace");
        fs::write(workspace.path().join("Cargo.toml"), "[package]").expect("manifest");

        let profile = ProjectProfile::detect(workspace.path());

        assert_eq!(profile.kind, ProjectKind::Rust);
        assert_eq!(profile.verification_command.as_deref(), Some("cargo test"));
    }

    #[test]
    fn detects_standard_library_python_tests() {
        let workspace = tempdir().expect("workspace");
        fs::create_dir(workspace.path().join("tests")).expect("tests directory");
        fs::write(
            workspace.path().join("tests/test_checkout.py"),
            "import unittest",
        )
        .expect("test file");

        let profile = ProjectProfile::detect(workspace.path());

        assert_eq!(profile.kind, ProjectKind::PythonUnittest);
        assert_eq!(
            profile.verification_command.as_deref(),
            Some("python -m unittest discover -s tests -v")
        );
    }

    #[test]
    fn uses_declared_node_test_script() {
        let workspace = tempdir().expect("workspace");
        fs::write(
            workspace.path().join("package.json"),
            serde_json::to_vec(&json!({"scripts": {"test": "vitest run"}})).expect("package json"),
        )
        .expect("manifest");

        let profile = ProjectProfile::detect(workspace.path());

        assert_eq!(profile.kind, ProjectKind::Node);
        assert_eq!(profile.verification_command.as_deref(), Some("npm test"));
    }

    #[test]
    fn reports_unknown_without_guessing_a_command() {
        let workspace = tempdir().expect("workspace");

        let profile = ProjectProfile::detect(workspace.path());

        assert_eq!(profile.kind, ProjectKind::Unknown);
        assert_eq!(profile.verification_command, None);
    }

    #[test]
    fn detects_pytest_from_each_supported_marker() {
        for (name, content, evidence) in [
            (
                "pyproject.toml",
                "[tool.pytest.ini_options]",
                "pyproject.toml",
            ),
            ("pytest.ini", "[pytest]", "pytest.ini"),
            ("tox.ini", "[testenv]", "tox.ini"),
            (
                "requirements.txt",
                "pytest>=8\n",
                "requirements.txt (pytest)",
            ),
        ] {
            let workspace = tempdir().expect("workspace");
            fs::write(workspace.path().join(name), content).expect("marker");

            let profile = ProjectProfile::detect(workspace.path());

            assert_eq!(profile.kind, ProjectKind::PythonPytest, "{name}");
            assert_eq!(profile.evidence, vec![evidence.to_owned()], "{name}");
            assert_eq!(
                profile.verification_command.as_deref(),
                Some("python -m pytest"),
                "{name}"
            );
        }
    }

    #[test]
    fn detects_maven_and_prefers_windows_wrapper() {
        let workspace = tempdir().expect("workspace");
        fs::write(workspace.path().join("pom.xml"), "<project/>").expect("manifest");
        fs::write(workspace.path().join("mvnw.cmd"), "").expect("wrapper");

        let profile = ProjectProfile::detect(workspace.path());

        assert_eq!(profile.kind, ProjectKind::Maven);
        assert_eq!(
            profile.verification_command.as_deref(),
            Some(r".\mvnw.cmd test")
        );
    }

    #[test]
    fn detects_gradle_kotlin_and_prefers_windows_wrapper() {
        let workspace = tempdir().expect("workspace");
        fs::write(workspace.path().join("build.gradle.kts"), "plugins {}").expect("manifest");
        fs::write(workspace.path().join("gradlew.bat"), "").expect("wrapper");

        let profile = ProjectProfile::detect(workspace.path());

        assert_eq!(profile.kind, ProjectKind::Gradle);
        assert_eq!(profile.evidence, vec!["build.gradle.kts"]);
        assert_eq!(
            profile.verification_command.as_deref(),
            Some(r".\gradlew.bat test")
        );
    }

    #[test]
    fn detects_go_project() {
        let workspace = tempdir().expect("workspace");
        fs::write(workspace.path().join("go.mod"), "module example.test/demo").expect("manifest");

        let profile = ProjectProfile::detect(workspace.path());

        assert_eq!(profile.kind, ProjectKind::Go);
        assert_eq!(
            profile.verification_command.as_deref(),
            Some("go test ./...")
        );
    }

    #[test]
    fn detects_dotnet_solution_and_project() {
        for marker in ["Demo.sln", "Demo.csproj"] {
            let workspace = tempdir().expect("workspace");
            fs::write(workspace.path().join(marker), "").expect("manifest");

            let profile = ProjectProfile::detect(workspace.path());

            assert_eq!(profile.kind, ProjectKind::DotNet, "{marker}");
            assert_eq!(profile.evidence, vec![marker.to_owned()], "{marker}");
            assert_eq!(profile.verification_command.as_deref(), Some("dotnet test"));
        }
    }

    #[test]
    fn node_falls_back_to_build_then_lint_without_a_real_test_script() {
        for (scripts, expected) in [
            (
                json!({"test": "echo Error: no test specified", "build": "vite build"}),
                Some("npm run build"),
            ),
            (json!({"lint": "eslint ."}), Some("npm run lint")),
            (json!({"start": "node app.js"}), None),
        ] {
            let workspace = tempdir().expect("workspace");
            fs::write(
                workspace.path().join("package.json"),
                serde_json::to_vec(&json!({"scripts": scripts})).expect("json"),
            )
            .expect("manifest");

            let profile = ProjectProfile::detect(workspace.path());

            assert_eq!(profile.kind, ProjectKind::Node);
            assert_eq!(profile.verification_command.as_deref(), expected);
        }
    }
}
