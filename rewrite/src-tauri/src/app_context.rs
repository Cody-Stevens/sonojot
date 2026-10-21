//! Exact local app identities become fixed layout hints. Never pass an app ID,
//! window title, document name or URL into a model prompt.
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Category {
    Mail,
    Tasks,
    Chat,
    Notes,
}

impl Category {
    pub fn name(self) -> &'static str {
        match self {
            Self::Mail => "mail",
            Self::Tasks => "tasks",
            Self::Chat => "chat",
            Self::Notes => "notes",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Mail => "an email app (Mail, Outlook or Gmail)",
            Self::Tasks => "a to-do or checklist app (Reminders, Things or Todoist)",
            Self::Chat => "a messaging app (Slack, Messages or Teams)",
            Self::Notes => "a notes or document app (Notes, Obsidian or Word)",
        }
    }

    pub(crate) fn from_description(description: &str) -> Option<Self> {
        [Self::Mail, Self::Tasks, Self::Chat, Self::Notes]
            .into_iter()
            .find(|category| category.description() == description)
    }
}

/// Case-insensitive exact aliases only: Windows executable basenames, macOS
/// bundle IDs and Linux window classes. Browsers never imply a web app.
pub fn category_for_identity(identity: &str) -> Option<Category> {
    use Category::*;
    match identity.to_ascii_lowercase().as_str() {
        "com.apple.mail"
        | "hxoutlook.exe"
        | "outlook.exe"
        | "olk.exe"
        | "com.microsoft.outlook"
        | "thunderbird.exe"
        | "thunderbird"
        | "org.mozilla.thunderbird"
        | "spark.exe"
        | "com.readdle.smartemail-mac"
        | "com.readdle.sparkdesktop"
        | "it.bloop.airmail"
        | "it.bloop.airmail2"
        | "com.bloop.airmail"
        | "com.bloop.airmail2" => Some(Mail),

        "com.apple.reminders"
        | "com.culturedcode.thingsmac"
        | "todoist.exe"
        | "todoist"
        | "com.todoist.mac.todoist"
        | "todo.exe"
        | "com.microsoft.to-do"
        | "ticktick.exe"
        | "ticktick"
        | "com.ticktick.task.mac"
        | "com.omnigroup.omnifocus"
        | "com.omnigroup.omnifocus2"
        | "com.omnigroup.omnifocus3"
        | "com.omnigroup.omnifocus4" => Some(Tasks),

        "slack.exe"
        | "slack"
        | "com.tinyspeck.slackmacgap"
        | "teams.exe"
        | "teams"
        | "ms-teams.exe"
        | "com.microsoft.teams"
        | "com.microsoft.teams2"
        | "com.apple.ichat"
        | "discord.exe"
        | "discord"
        | "com.hnc.discord"
        | "whatsapp.exe"
        | "net.whatsapp.whatsapp"
        | "telegram.exe"
        | "telegramdesktop"
        | "telegram-desktop"
        | "org.telegram.desktop"
        | "ru.keepcoder.telegram"
        | "signal.exe"
        | "signal"
        | "org.whispersystems.signal-desktop" => Some(Chat),

        "com.apple.notes"
        | "obsidian.exe"
        | "obsidian"
        | "md.obsidian"
        | "winword.exe"
        | "com.microsoft.word"
        | "com.apple.iwork.pages"
        | "onenote.exe"
        | "onenoteim.exe"
        | "com.microsoft.onenote"
        | "notion.exe"
        | "notion"
        | "notion.id"
        | "net.shinyfrog.bear"
        | "craft.exe"
        | "com.lukilabs.lukiapp"
        | "evernote.exe"
        | "com.evernote.evernote" => Some(Notes),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn common_native_apps_have_exact_categories() {
        for (category, identities) in [
            (
                Category::Mail,
                &[
                    "com.apple.mail",
                    "HxOutlook.exe",
                    "OUTLOOK.EXE",
                    "olk.exe",
                    "com.microsoft.Outlook",
                    "thunderbird.exe",
                    "org.mozilla.thunderbird",
                    "Spark.exe",
                    "com.readdle.smartemail-Mac",
                    "com.readdle.SparkDesktop",
                    "it.bloop.airmail2",
                    "com.bloop.airmail2",
                ][..],
            ),
            (
                Category::Tasks,
                &[
                    "com.apple.reminders",
                    "com.culturedcode.ThingsMac",
                    "Todoist.exe",
                    "com.todoist.mac.Todoist",
                    "ToDo.exe",
                    "com.microsoft.to-do",
                    "TickTick.exe",
                    "com.TickTick.task.mac",
                    "com.omnigroup.OmniFocus3",
                    "com.omnigroup.OmniFocus4",
                ][..],
            ),
            (
                Category::Chat,
                &[
                    "slack.exe",
                    "com.tinyspeck.slackmacgap",
                    "Teams.exe",
                    "ms-teams.exe",
                    "com.microsoft.teams",
                    "com.microsoft.teams2",
                    "com.apple.iChat",
                    "Discord.exe",
                    "com.hnc.Discord",
                    "WhatsApp.exe",
                    "net.whatsapp.WhatsApp",
                    "Telegram.exe",
                    "org.telegram.desktop",
                    "ru.keepcoder.Telegram",
                    "Signal.exe",
                    "org.whispersystems.signal-desktop",
                ][..],
            ),
            (
                Category::Notes,
                &[
                    "com.apple.Notes",
                    "Obsidian.exe",
                    "md.obsidian",
                    "WINWORD.EXE",
                    "com.microsoft.Word",
                    "com.apple.iWork.Pages",
                    "ONENOTE.EXE",
                    "onenoteim.exe",
                    "com.microsoft.onenote",
                    "Notion.exe",
                    "notion.id",
                    "net.shinyfrog.bear",
                    "Craft.exe",
                    "com.lukilabs.lukiapp",
                    "Evernote.exe",
                    "com.evernote.Evernote",
                ][..],
            ),
        ] {
            for identity in identities {
                assert_eq!(
                    category_for_identity(identity),
                    Some(category),
                    "{identity}"
                );
            }
        }
        for (identity, category) in [
            ("Thunderbird", Category::Mail),
            ("todoist", Category::Tasks),
            ("TickTick", Category::Tasks),
            ("Slack", Category::Chat),
            ("discord", Category::Chat),
            ("TelegramDesktop", Category::Chat),
            ("Signal", Category::Chat),
            ("obsidian", Category::Notes),
        ] {
            assert_eq!(category_for_identity(identity), Some(category));
        }
    }

    #[test]
    fn browsers_terminals_editors_and_lookalikes_never_supply_a_hint() {
        for identity in [
            "",
            "chrome.exe",
            "msedge.exe",
            "firefox.exe",
            "brave.exe",
            "com.google.Chrome",
            "com.apple.Safari",
            "org.mozilla.firefox",
            "Google-chrome",
            "chromium",
            "WindowsTerminal.exe",
            "cmd.exe",
            "powershell.exe",
            "com.apple.Terminal",
            "com.googlecode.iterm2",
            "Alacritty",
            "kitty",
            "Code.exe",
            "com.microsoft.VSCode",
            "code",
            "Cursor.exe",
            "notepad.exe",
            "com.apple.TextEdit",
            "unknown.exe",
            "outlook.exe.bak",
            "my-slack-client",
            "https://mail.google.com",
            "Gmail - Google Chrome",
            "C:\\Apps\\outlook.exe",
            " slack.exe ",
        ] {
            assert_eq!(category_for_identity(identity), None, "{identity}");
        }
    }

    #[test]
    fn context_contains_only_the_evaluated_descriptions() {
        for (category, name, description) in [
            (
                Category::Mail,
                "mail",
                "an email app (Mail, Outlook or Gmail)",
            ),
            (
                Category::Tasks,
                "tasks",
                "a to-do or checklist app (Reminders, Things or Todoist)",
            ),
            (
                Category::Chat,
                "chat",
                "a messaging app (Slack, Messages or Teams)",
            ),
            (
                Category::Notes,
                "notes",
                "a notes or document app (Notes, Obsidian or Word)",
            ),
        ] {
            assert_eq!(category.name(), name);
            assert_eq!(category.description(), description);
            assert_eq!(Category::from_description(description), Some(category));
            assert_eq!(serde_json::to_value(category).unwrap(), name);
        }
        assert_eq!(Category::from_description("outlook.exe"), None);
    }
}
