//! Shared slot vocabulary for the desktop owner and its gallery worker.
use super::library::{CharacterCatalog, CharacterEntry};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub(crate) const MAX_CHARACTERS: usize = 8;

#[derive(Message, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum RosterRequest {
    AddSlot,
    Assign { slot: usize, path: String },
    Clear { slot: usize },
    Import { slot: usize, path: PathBuf },
    Remove { path: String },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SlotView {
    pub(crate) used: bool,
    pub(crate) path: Option<String>,
    pub(crate) name: String,
}

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct CharacterRoster {
    pub(crate) models: Vec<CharacterEntry>,
    pub(crate) slots: Vec<SlotView>,
    pub(crate) can_add: bool,
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) selection_notice: (u64, String),
    #[serde(default)]
    pub(crate) thumbnail_revision: u64,
}

impl Default for CharacterRoster {
    fn default() -> Self {
        Self {
            models: CharacterCatalog::default().0,
            slots: vec![
                SlotView {
                    used: true,
                    path: Some(super::DEFAULT_CHARACTER.into()),
                    name: "Nera".into(),
                },
                SlotView {
                    used: false,
                    path: None,
                    name: String::new(),
                },
            ],
            can_add: true,
            status: String::new(),
            selection_notice: Default::default(),
            thumbnail_revision: 0,
        }
    }
}

impl RosterRequest {
    pub(crate) fn valid(&self) -> bool {
        match self {
            Self::AddSlot => true,
            Self::Assign { slot, path } => {
                *slot < MAX_CHARACTERS
                    && (path == super::DEFAULT_CHARACTER || super::saved_filename(path).is_some())
            }
            Self::Clear { slot } => *slot < MAX_CHARACTERS,
            Self::Remove { path } => super::saved_filename(path).is_some(),
            Self::Import { slot, path } => {
                *slot < MAX_CHARACTERS
                    && path.is_absolute()
                    && path
                        .extension()
                        .is_some_and(|extension| extension.eq_ignore_ascii_case("vrm"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn remove_protocol_round_trips_owned_imports_but_rejects_builtin_and_source_paths() {
        let request = RosterRequest::Remove {
            path: "characters://model-abc123.vrm".into(),
        };
        let decoded: RosterRequest =
            serde_json::from_str(&serde_json::to_string(&request).unwrap()).unwrap();
        assert_eq!(decoded, request);
        assert!(decoded.valid());
        for path in [
            super::super::DEFAULT_CHARACTER,
            "C:/original.vrm",
            "characters://../original.vrm",
        ] {
            assert!(!RosterRequest::Remove { path: path.into() }.valid());
        }
    }
    #[test]
    fn slot_commands_round_trip_and_reject_out_of_range_or_external_assets() {
        let request = RosterRequest::Assign {
            slot: 7,
            path: super::super::DEFAULT_CHARACTER.into(),
        };
        assert!(
            serde_json::from_str::<RosterRequest>(&serde_json::to_string(&request).unwrap())
                .unwrap()
                .valid()
        );
        assert!(!RosterRequest::Clear { slot: 8 }.valid());
        assert!(
            !RosterRequest::Assign {
                slot: 1,
                path: "C:/private.vrm".into()
            }
            .valid()
        );
    }
}
