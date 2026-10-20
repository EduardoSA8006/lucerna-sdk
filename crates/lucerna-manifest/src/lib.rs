#![warn(missing_docs)]
//! Manifesto JSON dos apps do Lucerna: leitura, validação e compatibilidade
//! da `api` do painel com a do shell.

use std::{collections::BTreeMap, path::PathBuf};

use lucerna_protocol::Version;
use serde::{Deserialize, Serialize};

/// Versão do formato do manifesto aceita por este crate.
pub const SCHEMA: u32 = 1;

/// Manifesto de um app (`<app>.json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    /// Versão do formato; só `1` é aceito.
    pub schema: u32,
    /// Identificação do app.
    pub app: AppInfo,
    /// Painel QML, se o app tiver um.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub panel: Option<PanelInfo>,
    /// Cartão QML, se o app tiver um (exige painel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card: Option<CardInfo>,
    /// Atalhos sugeridos (nome → combinação).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub shortcuts: BTreeMap<String, String>,
}

/// Dados do app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AppInfo {
    /// Id do app (`^[a-z][a-z0-9-]{0,31}$`).
    pub id: String,
    /// Nome para exibição.
    pub name: String,
    /// Nome do ícone.
    pub icon: String,
    /// Nome do arquivo do socket: `<id>.sock`.
    pub socket: String,
    /// Comando da interface de terminal, se houver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tui: Option<String>,
}

/// Painel QML do app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PanelInfo {
    /// Caminho absoluto do arquivo QML.
    pub qml: PathBuf,
    /// Versão da API do shell para a qual o painel foi escrito.
    pub api: Version,
}

/// Cartão QML do app.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CardInfo {
    /// Caminho absoluto do arquivo QML.
    pub qml: PathBuf,
}

/// Erro ao ler ou validar um manifesto.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ManifestError {
    /// JSON inválido, campo faltando ou versão de `api` malformada.
    #[error("manifesto inválido: {0}")]
    Json(String),
    /// `schema` diferente de 1.
    #[error("schema do manifesto não suportado: {0} (esperado {SCHEMA})")]
    Schema(u32),
    /// Id do app fora do formato.
    #[error("id de app inválido: {0}")]
    AppId(String),
    /// `socket` diferente de `<id>.sock`.
    #[error("socket do manifesto é {found:?}, esperado {expected:?}")]
    Socket {
        /// Valor esperado (`<id>.sock`).
        expected: String,
        /// Valor encontrado no manifesto.
        found: String,
    },
    /// Campo de texto vazio.
    #[error("o campo {0} não pode ser vazio")]
    EmptyField(&'static str),
    /// Caminho de QML que não é absoluto.
    #[error("o caminho {} precisa ser absoluto", .0.display())]
    RelativePath(PathBuf),
    /// `card` sem `panel`.
    #[error("o cartão exige um painel")]
    CardWithoutPanel,
}

impl ManifestError {
    /// Rótulo estável do tipo do erro (usado nos exemplos compartilhados).
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Json(_) => "json",
            Self::Schema(_) => "schema",
            Self::AppId(_) => "app-id",
            Self::Socket { .. } => "socket",
            Self::EmptyField(_) => "field",
            Self::RelativePath(_) => "path",
            Self::CardWithoutPanel => "card-without-panel",
        }
    }
}

impl Manifest {
    /// Lê e valida um manifesto a partir do JSON.
    pub fn from_json(s: &str) -> Result<Manifest, ManifestError> {
        let m: Manifest =
            serde_json::from_str(s).map_err(|e| ManifestError::Json(e.to_string()))?;
        m.validate()?;
        Ok(m)
    }

    /// Confere as regras do manifesto.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.schema != SCHEMA {
            return Err(ManifestError::Schema(self.schema));
        }
        lucerna_paths::validate_app_id(&self.app.id)
            .map_err(|_| ManifestError::AppId(self.app.id.clone()))?;
        let fields = [("name", &self.app.name), ("icon", &self.app.icon)];
        for (nome, valor) in fields {
            if valor.is_empty() {
                return Err(ManifestError::EmptyField(nome));
            }
        }
        if self.app.tui.as_deref() == Some("") {
            return Err(ManifestError::EmptyField("tui"));
        }
        let expected = format!("{}.sock", self.app.id);
        if self.app.socket != expected {
            return Err(ManifestError::Socket {
                expected,
                found: self.app.socket.clone(),
            });
        }
        let qmls = self
            .panel
            .iter()
            .map(|p| &p.qml)
            .chain(self.card.iter().map(|c| &c.qml));
        for qml in qmls {
            if !qml.is_absolute() {
                return Err(ManifestError::RelativePath(qml.clone()));
            }
        }
        if self.card.is_some() && self.panel.is_none() {
            return Err(ManifestError::CardWithoutPanel);
        }
        Ok(())
    }

    /// Serializa em JSON indentado (partes opcionais vazias são omitidas).
    pub fn to_json_pretty(&self) -> String {
        serde_json::to_string_pretty(self).expect("o manifesto sempre serializa")
    }
}

/// O painel carrega quando tem o mesmo major e `panel.minor <= shell.minor`.
pub fn api_compatible(panel: Version, shell: Version) -> bool {
    panel.major == shell.major && panel.minor <= shell.minor
}
