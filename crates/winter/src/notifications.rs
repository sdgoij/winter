//! Desktop notifications a program asked for.
//!
//! urxvt's OSC 777 is the source today; the adapter queues it once the
//! application asks for its `DesktopNotification` feature, which `Pane` does.
//! Showing one is best effort: a machine with no notification service still
//! runs the terminal.

use notify_rust::Notification;

/// The text a program asked to show.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// The summary line.
    pub title: String,
    /// The body, which may be empty.
    pub body: String,
}

/// The message a notification sequence carried.
///
/// A blank title has nothing to identify the notification, so it gets the
/// application's name. Surrounding whitespace is dropped from both parts.
pub fn message(title: &str, body: &str) -> Message {
    let title = title.trim();
    Message {
        title: if title.is_empty() {
            "Winter".to_string()
        } else {
            title.to_string()
        },
        body: body.trim().to_string(),
    }
}

/// Shows the message, dropping a platform that has no notification service.
pub fn show(title: &str, body: &str) {
    let message = message(title, body);
    // Windows needs an application id and an unpackaged binary has none of its
    // own; `notify-rust` falls back to PowerShell's, the usual workaround. A
    // machine with no notification service drops the call.
    let _ = Notification::new()
        .summary(&message.title)
        .body(&message.body)
        .appname("Winter")
        .show();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_notification_keeps_its_title_and_body() {
        assert_eq!(
            message("Build", "finished with warnings"),
            Message {
                title: "Build".to_string(),
                body: "finished with warnings".to_string(),
            }
        );
    }

    #[test]
    fn a_blank_title_gets_the_application_name() {
        assert_eq!(message("  ", "body").title, "Winter");
        assert_eq!(
            message(" Build ", " body "),
            Message {
                title: "Build".to_string(),
                body: "body".to_string(),
            }
        );
    }
}
