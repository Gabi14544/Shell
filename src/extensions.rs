use gtk::{gio, glib, prelude::*};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

const PYTHON_SERVER: &str = "python-lsp-server==1.15.0";
const RUFF_PLUGIN: &str = "python-lsp-ruff==2.3.4";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Extension {
    pub id: String,
    pub name: String,
    pub language: String,
    pub suffixes: Vec<String>,
    pub command: Vec<String>,
    #[serde(default)]
    pub settings: Value,
}

impl Extension {
    pub fn matches(&self, path: &Path) -> bool {
        let Some(suffix) = path.extension().and_then(|value| value.to_str()) else {
            return false;
        };
        self.suffixes
            .iter()
            .any(|item| item.eq_ignore_ascii_case(suffix))
    }

    pub fn command(&self) -> Command {
        let executable = if self.id == "python" {
            python_env().join("bin/pylsp").into_os_string()
        } else {
            self.command[0].clone().into()
        };
        let mut command = Command::new(executable);
        command.args(&self.command[1..]);
        command
    }
}

pub fn catalog() -> Vec<Extension> {
    vec![
        Extension {
            id: "python".into(),
            name: "Python · Jedi y Ruff".into(),
            language: "python".into(),
            suffixes: vec!["py".into(), "pyi".into()],
            command: vec!["pylsp".into()],
            settings: json!({"pylsp": {"plugins": {
                "ruff": {"enabled": true},
                "pycodestyle": {"enabled": false},
                "pyflakes": {"enabled": false},
                "mccabe": {"enabled": false},
                "autopep8": {"enabled": false},
                "jedi_completion": {"include_params": false}
            }}}),
        },
        system(
            "rust",
            "Rust · rust-analyzer",
            "rust",
            &["rs"],
            &["rust-analyzer"],
        ),
        system(
            "cpp",
            "C y C++ · clangd",
            "cpp",
            &["c", "h", "cc", "cpp", "cxx", "hpp", "hxx"],
            &["clangd", "--background-index=false", "-j=1"],
        ),
    ]
}

fn system(id: &str, name: &str, language: &str, suffixes: &[&str], command: &[&str]) -> Extension {
    Extension {
        id: id.into(),
        name: name.into(),
        language: language.into(),
        suffixes: suffixes.iter().map(|s| (*s).into()).collect(),
        command: command.iter().map(|s| (*s).into()).collect(),
        settings: Value::Null,
    }
}

fn data_dir() -> PathBuf {
    glib::user_data_dir().join("brasa")
}

fn python_env() -> PathBuf {
    data_dir().join("python-lsp")
}

pub fn python_ready() -> bool {
    let env = python_env();
    env.join(".instalado-0.1").is_file() && env.join("bin/pylsp").is_file()
}

pub fn install_python() -> Result<(), String> {
    if python_ready() {
        return Ok(());
    }
    let env = python_env();
    fs::create_dir_all(data_dir()).map_err(|err| err.to_string())?;
    run(Command::new("python3").args(["-m", "venv"]).arg(&env)).map_err(|err| {
        format!("No se pudo crear el entorno de Python. Instalá python3-venv.\n{err}")
    })?;
    run(Command::new(env.join("bin/python")).args([
        "-m",
        "pip",
        "--isolated",
        "--disable-pip-version-check",
        "--no-input",
        "--timeout",
        "15",
        "--retries",
        "1",
        "install",
        "--no-cache-dir",
        PYTHON_SERVER,
        RUFF_PLUGIN,
    ]))?;
    fs::write(env.join(".instalado-0.1"), []).map_err(|err| err.to_string())
}

fn run(command: &mut Command) -> Result<(), String> {
    let output = command.output().map_err(|err| err.to_string())?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);
    let details = if stderr.trim().is_empty() {
        &stdout
    } else {
        &stderr
    };
    Err(details
        .lines()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n"))
}

pub fn load() -> Result<Vec<Extension>, String> {
    let path = data_dir().join("extensiones.json");
    let json = match fs::read(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err.to_string()),
    };
    let list: Vec<Extension> = serde_json::from_slice(&json).map_err(|err| err.to_string())?;
    let catalog = catalog();
    list.into_iter()
        .map(|extension| {
            if let Some(item) = catalog.iter().find(|item| item.id == extension.id) {
                return Ok(item.clone());
            }
            validate(&extension)?;
            Ok(extension)
        })
        .collect()
}

pub fn save(extensions: &[Extension]) -> Result<(), String> {
    let dir = data_dir();
    fs::create_dir_all(&dir).map_err(|err| err.to_string())?;
    let bytes = serde_json::to_vec_pretty(extensions).map_err(|err| err.to_string())?;
    gio::File::for_path(dir.join("extensiones.json"))
        .replace_contents(
            &bytes,
            None,
            false,
            gio::FileCreateFlags::PRIVATE,
            gio::Cancellable::NONE,
        )
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn validate(extension: &Extension) -> Result<(), String> {
    if extension.name.trim().is_empty()
        || extension.language.is_empty()
        || extension.suffixes.is_empty()
        || extension.command.first().is_none_or(String::is_empty)
    {
        return Err(
            "Completá el nombre, el ID del lenguaje, las extensiones de archivo y el comando."
                .into(),
        );
    }
    if extension.suffixes.iter().any(|suffix| {
        suffix.is_empty()
            || !suffix
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_' || ch == '-')
    }) {
        return Err("Usá extensiones como js, ts o go, separadas por comas.".into());
    }
    Ok(())
}

pub fn custom(
    name: &str,
    language: &str,
    suffixes: &str,
    command: &str,
) -> Result<Extension, String> {
    let argv = glib::shell_parse_argv(command).map_err(|err| err.to_string())?;
    let extension = Extension {
        id: format!("local-{}", glib::uuid_string_random()),
        name: name.trim().into(),
        language: language.trim().into(),
        suffixes: suffixes
            .split(',')
            .map(|s| s.trim().trim_start_matches('.').to_lowercase())
            .filter(|s| !s.is_empty())
            .collect(),
        command: argv
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect(),
        settings: Value::Null,
    };
    validate(&extension)?;
    if glib::find_program_in_path(&extension.command[0]).is_none() {
        return Err(format!(
            "No se encontró {}. Indicá su ruta completa o agregalo al PATH.",
            extension.command[0]
        ));
    }
    Ok(extension)
}

pub fn workspace(path: &Path) -> PathBuf {
    let parent = path.parent().unwrap_or(path);
    for dir in parent.ancestors() {
        if [
            ".git",
            "pyproject.toml",
            "Cargo.toml",
            "package.json",
            "go.mod",
            "compile_commands.json",
            ".clangd",
        ]
        .iter()
        .any(|marker| dir.join(marker).exists())
        {
            return dir.to_path_buf();
        }
    }
    parent.to_path_buf()
}

pub fn language_id(extension: &Extension, path: &Path) -> String {
    if extension.id == "cpp" && path.extension().is_some_and(|ext| ext == "c") {
        return "c".into();
    }
    extension.language.clone()
}
