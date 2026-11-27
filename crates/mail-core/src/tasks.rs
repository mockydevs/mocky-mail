//! Tasks (to-dos): the fields the app shows and edits, whichever service
//! keeps them (a `CalDAV` `VTODO`, Google Tasks, Microsoft To Do, or this
//! device), and checking what the editor sends.

use serde::{Deserialize, Serialize};

use crate::importance::Importance;

/// Longest task title kept.
pub const MAX_TITLE: usize = 500;
/// Longest task notes kept.
pub const MAX_NOTES: usize = 8000;
/// Most categories on one task.
pub const MAX_CATEGORIES: usize = 20;

/// A task's own fields.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TaskFields {
    /// What to do.
    pub title: String,
    /// Notes.
    pub notes: String,
    /// When it is due (Unix seconds; a day is midnight UTC).
    pub due: Option<i64>,
    /// When to be reminded (Unix seconds).
    pub reminder: Option<i64>,
    /// Low, normal or high.
    pub importance: Importance,
    /// Done.
    pub completed: bool,
    /// When it was done.
    pub completed_at: Option<i64>,
    /// Category names.
    pub categories: Vec<String>,
}

fn tidy(text: &str, max: usize, lines: bool) -> String {
    text.chars()
        .filter(|c| !c.is_control() || (lines && *c == '\n'))
        .take(max)
        .collect::<String>()
        .trim()
        .to_owned()
}

/// Tidy and check a task from the editor. `now` fills in when a newly
/// completed task was done; `was` is the task before the edit, if any.
///
/// # Errors
/// A message for the user.
pub fn checked(
    mut task: TaskFields,
    was: Option<&TaskFields>,
    now: i64,
) -> Result<TaskFields, &'static str> {
    // 1970 to 2200: anything else is a mistake.
    const LATEST: i64 = 7_258_118_400;
    task.title = tidy(&task.title, MAX_TITLE, false);
    task.notes = tidy(&task.notes.replace("\r\n", "\n"), MAX_NOTES, true);
    if task.title.is_empty() {
        return Err("Give the task a title.");
    }
    if [task.due, task.reminder]
        .into_iter()
        .flatten()
        .any(|t| !(0..LATEST).contains(&t))
    {
        return Err("Choose the date again.");
    }
    let mut names: Vec<String> = Vec::new();
    for name in &task.categories {
        let name = tidy(name, 100, false);
        if !name.is_empty() && !names.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            names.push(name);
        }
    }
    names.truncate(MAX_CATEGORIES);
    task.categories = names;
    task.completed_at = if task.completed {
        was.filter(|w| w.completed)
            .and_then(|w| w.completed_at)
            .or(task.completed_at)
            .or(Some(now))
    } else {
        None
    };
    Ok(task)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn editor_tasks_are_tidied() {
        let task = TaskFields {
            title: "  Call Kamau\u{0} ".into(),
            notes: "a\r\nb".into(),
            categories: vec!["Red".into(), "red".into(), " ".into()],
            completed: true,
            ..TaskFields::default()
        };
        let done = checked(task.clone(), None, 50).unwrap();
        assert_eq!(done.title, "Call Kamau");
        assert_eq!(done.notes, "a\nb");
        assert_eq!(done.categories, ["Red"]);
        assert_eq!(done.completed_at, Some(50));
        // Done earlier: the time it was done stays.
        let again = checked(task.clone(), Some(&done), 99).unwrap();
        assert_eq!(again.completed_at, Some(50));
        // Not done: no time.
        let open = checked(
            TaskFields {
                completed: false,
                ..task.clone()
            },
            Some(&done),
            99,
        )
        .unwrap();
        assert_eq!(open.completed_at, None);
        assert!(
            checked(
                TaskFields {
                    title: " ".into(),
                    ..task.clone()
                },
                None,
                0
            )
            .is_err()
        );
        assert!(
            checked(
                TaskFields {
                    due: Some(-1),
                    ..task
                },
                None,
                0
            )
            .is_err()
        );
    }
}
