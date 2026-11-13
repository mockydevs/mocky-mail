//! Small key/value app settings.

use rusqlite::{OptionalExtension, params};

use crate::{Result, Store};

impl Store {
    /// The stored value for `key`, if any.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn get_setting(&self, key: &str) -> Result<Option<String>> {
        Ok(self
            .conn()
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Store `value` under `key`, replacing any previous value.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn set_setting(&self, key: &str, value: &str) -> Result<()> {
        self.conn().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT (key) DO UPDATE SET value = excluded.value",
            params![key, value],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use crate::Store;

    #[test]
    fn missing_setting_is_none() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.get_setting("theme").unwrap(), None);
    }

    #[test]
    fn set_then_overwrite() {
        let store = Store::open_in_memory().unwrap();
        store.set_setting("theme", "light").unwrap();
        store.set_setting("theme", "dark").unwrap();
        store.set_setting("remote_images", "block").unwrap();
        assert_eq!(store.get_setting("theme").unwrap().as_deref(), Some("dark"));
        assert_eq!(
            store.get_setting("remote_images").unwrap().as_deref(),
            Some("block")
        );
    }
}
