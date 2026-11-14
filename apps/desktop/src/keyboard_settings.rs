use serde::{Deserialize, Serialize};
use staminator_keyboard_schema::{Finger, valid_physical_id};
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Read, Write},
    path::Path,
};

pub type Assignments = BTreeMap<String, Finger>;
const MAX_SCHEME_BYTES: u64 = 131_072;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct FingerScheme {
    pub version: u32,
    pub name: String,
    pub assignments: Assignments,
}

impl FingerScheme {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.name.trim().is_empty()
            || self.name.len() > 128
            || self.assignments.len() > 256
            || self.assignments.keys().any(|id| !valid_physical_id(id))
        {
            return Err("Invalid finger scheme".into());
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self, String> {
        let file = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take(MAX_SCHEME_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() as u64 > MAX_SCHEME_BYTES {
            return Err("Finger scheme is too large".into());
        }
        let scheme: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        scheme.validate()?;
        Ok(scheme)
    }
    pub fn export(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        let data = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| e.to_string())?;
        if let Err(e) = f.write_all(&data).and_then(|_| f.flush()) {
            drop(f);
            let _ = fs::remove_file(path);
            return Err(e.to_string());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Preferences {
    pub keyboard: String,
    pub finger_scheme: String,
    pub schemes: BTreeMap<String, FingerScheme>,
}

impl Preferences {
    pub fn decode(data: &str) -> Self {
        let mut value: Self = serde_json::from_str(data).unwrap_or_default();
        value
            .schemes
            .retain(|name, scheme| name == &scheme.name && scheme.validate().is_ok());
        value
    }
    pub fn encode(&self) -> io::Result<String> {
        serde_json::to_string(self).map_err(io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finger_scheme_export_and_import_preserve_assignments_and_existing_file() {
        let path = std::env::temp_dir().join(format!(
            "staminator-fingers-{}-{}.json",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let scheme = FingerScheme {
            version: 1,
            name: "Personal".into(),
            assignments: [
                ("Digit4".into(), Finger::LeftMiddle),
                ("Digit6".into(), Finger::LeftIndex),
            ]
            .into(),
        };
        scheme.export(&path).unwrap();
        assert_eq!(FingerScheme::load(&path).unwrap(), scheme);
        assert!(scheme.export(&path).is_err());
        assert_eq!(FingerScheme::load(&path).unwrap(), scheme);
        fs::write(&path, vec![b' '; MAX_SCHEME_BYTES as usize + 1]).unwrap();
        assert!(FingerScheme::load(&path).is_err());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn corrupted_finger_schemes_do_not_replace_valid_preferences() {
        let data = r#"{"keyboard":"one","finger_scheme":"safe","schemes":{"safe":{"version":1,"name":"safe","assignments":{"Digit4":"left_middle"}},"bad":{"version":9,"name":"bad","assignments":{}},"unknown":{"version":1,"name":"unknown","assignments":{"Key1":"left_index"}}}}"#;
        let prefs = Preferences::decode(data);
        assert_eq!(prefs.keyboard, "one");
        assert_eq!(prefs.schemes.len(), 1);
        assert_eq!(
            prefs.schemes["safe"].assignments["Digit4"],
            Finger::LeftMiddle
        );
        assert!(!prefs.schemes.contains_key("bad"));
        assert!(!prefs.schemes.contains_key("unknown"));
    }

    #[test]
    fn invalid_schemes_are_rejected_and_profile_preferences_recover() {
        let mut scheme = FingerScheme {
            version: 1,
            name: "Custom".into(),
            assignments: [("Digit4".into(), Finger::LeftMiddle)].into(),
        };
        assert!(scheme.validate().is_ok());
        let mut prefs = Preferences::default();
        prefs.schemes.insert(scheme.name.clone(), scheme.clone());
        assert_eq!(Preferences::decode(&prefs.encode().unwrap()), prefs);
        scheme.assignments.insert("Key1".into(), Finger::LeftMiddle);
        assert!(scheme.validate().is_err());
        assert!(Preferences::decode("not json").schemes.is_empty());
    }
}
