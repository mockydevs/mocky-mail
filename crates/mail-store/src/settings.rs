//! Small key/value app settings.

use rusqlite::{OptionalExtension, params};

use crate::{Result, Store};

impl Store {
    /// Set (`Some`) or clear (`None`) the Focused Inbox rule for a sender
    /// key: an address, or `@domain`.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn set_focus_rule(
        &self,
        sender: &str,
        focus: Option<mail_core::focus::Focus>,
    ) -> Result<()> {
        match focus {
            Some(focus) => self.conn().execute(
                "INSERT INTO focus_rules (sender, focus) VALUES (?1, ?2)
                 ON CONFLICT (sender) DO UPDATE SET focus = excluded.focus",
                params![sender, focus.as_str()],
            )?,
            None => self
                .conn()
                .execute("DELETE FROM focus_rules WHERE sender = ?1", params![sender])?,
        };
        Ok(())
    }

    /// The user's Focused Inbox rules: sender key and where it goes.
    ///
    /// # Errors
    /// Fails on a database error.
    pub fn focus_rules(&self) -> Result<Vec<(String, mail_core::focus::Focus)>> {
        let conn = self.conn();
        let mut stmt = conn.prepare("SELECT sender, focus FROM focus_rules ORDER BY sender")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (sender, focus) = row?;
            if let Some(focus) = mail_core::focus::Focus::parse(&focus) {
                out.push((sender, focus));
            }
        }
        Ok(out)
    }

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
