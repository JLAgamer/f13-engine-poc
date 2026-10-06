//! Blueprint class descriptions written by `tools/f13-export` (`classes.json`): each class's Blueprint parents, its
//! first native parent, its defaults, and its components in actor space.

use crate::level::{null_as_default, str_list, ComponentInfo, Xform};
use f13_core::spawn::ClassQuery;
use serde::Deserialize;
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct ClassInfo {
    pub name: String,
    #[serde(default)]
    pub path: Option<String>,
    /// First native (C++) class in the parent chain; for a native class, itself.
    #[serde(default)]
    pub native: Option<String>,
    /// Blueprint parents, nearest first.
    #[serde(default, deserialize_with = "null_as_default")]
    pub supers: Vec<String>,
    #[serde(default)]
    pub props: serde_json::Value,
    #[serde(default, deserialize_with = "null_as_default")]
    pub components: Vec<ComponentInfo>,
}

use crate::native_classes::NATIVE_PARENTS;

#[derive(Debug, Clone, Default)]
pub struct ClassDb {
    pub classes: HashMap<String, ClassInfo>,
}

impl ClassDb {
    pub fn load(path: &Path) -> Result<Self, crate::LoadError> {
        let text = std::fs::read_to_string(path).map_err(crate::LoadError::Io)?;
        let classes = serde_json::from_str(&text).map_err(crate::LoadError::Json)?;
        Ok(Self { classes })
    }

    pub fn get(&self, class: &str) -> Option<&ClassInfo> {
        self.classes.get(class)
    }

    /// `class` is `parent`, has it as a Blueprint parent, or has it (or a known native subclass of it) as native
    /// parent. Works for component classes too.
    pub fn is_child_of(&self, class: &str, parent: &str) -> bool {
        if class == parent {
            return true;
        }
        let native = match self.classes.get(class) {
            Some(c) => {
                if c.supers.iter().any(|s| s == parent) {
                    return true;
                }
                c.native.clone().unwrap_or_else(|| class.to_string())
            }
            None => class.to_string(),
        };
        let mut n = native.as_str();
        for _ in 0..16 {
            if n == parent {
                return true;
            }
            match NATIVE_PARENTS.iter().find(|(c, _)| *c == n) {
                Some((_, p)) => n = p,
                None => return false,
            }
        }
        false
    }

    /// The class's root component transform relative to the spawn transform.
    pub fn root_relative(&self, class: &str) -> Xform {
        self.get(class)
            .and_then(|c| c.components.iter().find(|x| x.root).and_then(|x| x.root_relative))
            .unwrap_or(Xform::IDENTITY)
    }

    pub fn prop<'a>(&'a self, class: &str, name: &str) -> &'a serde_json::Value {
        static NULL: serde_json::Value = serde_json::Value::Null;
        self.get(class).map(|c| &c.props[name]).unwrap_or(&NULL)
    }

    pub fn display_name(&self, class: &str) -> String {
        self.prop(class, "FriendlyName")["LocalizedString"].as_str().map(String::from).unwrap_or_else(|| class.trim_end_matches("_C").to_string())
    }
}

impl ClassQuery for ClassDb {
    fn is_child_of(&self, class: &str, parent: &str) -> bool {
        ClassDb::is_child_of(self, class, parent)
    }

    fn is_large(&self, class: &str) -> bool {
        // Weapons (guns too) and traps are carried in the hands by their native constructors; Blueprints only store
        // bIsLarge when they differ from their parent.
        self.prop(class, "bIsLarge").as_bool().unwrap_or_else(|| self.is_child_of(class, "SCWeapon") || self.is_child_of(class, "SCTrap"))
    }

    fn required_parts(&self, vehicle_class: &str) -> Vec<String> {
        let Some(c) = self.get(vehicle_class) else { return vec![] };
        c.components
            .iter()
            .filter(|x| self.is_child_of(&x.kind, "SCRepairComponent"))
            .flat_map(|x| str_list(&x.props["RequiredPartClasses"]))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(name: &str, native: &str, supers: &[&str], props: serde_json::Value) -> ClassInfo {
        ClassInfo {
            name: name.into(),
            path: None,
            native: Some(native.into()),
            supers: supers.iter().map(|s| s.to_string()).collect(),
            props,
            components: vec![],
        }
    }

    #[test]
    fn hierarchy_follows_blueprint_and_native_parents() {
        let mut db = ClassDb::default();
        db.classes.insert("Dresser_C".into(), class("Dresser_C", "SCCabinet", &["Base_C"], serde_json::Value::Null));
        db.classes.insert("Double_C".into(), class("Double_C", "SCDoubleCabinet", &[], serde_json::Value::Null));
        db.classes.insert("Keys_C".into(), class("Keys_C", "SCRepairPart", &[], serde_json::json!({"bIsLarge": false})));
        assert!(db.is_child_of("Dresser_C", "Base_C"));
        assert!(db.is_child_of("Dresser_C", "SCCabinet"));
        assert!(db.is_child_of("Double_C", "SCCabinet"));
        assert!(db.is_child_of("Keys_C", "SCItem"));
        assert!(!db.is_child_of("Keys_C", "SCCabinet"));
        assert!(db.is_child_of("SCVehicleStarterComponent", "SCRepairComponent"));
        assert!(!ClassQuery::is_large(&db, "Keys_C"));
    }
}
