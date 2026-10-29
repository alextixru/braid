//! Автозапуск при входе (ист. 55): LaunchAgent `~/Library/LaunchAgents/<label>.plist`
//! с `RunAtLoad`. Файл пишется и удаляется платформой; `launchctl` —
//! по возможности, чтобы изменение действовало без перелогина.

use std::path::Path;

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Текст plist агента: запускает `program` при входе, не перезапускает.
pub fn launch_agent_plist(label: &str, program: &Path) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \t<key>Label</key>\n\
         \t<string>{label}</string>\n\
         \t<key>ProgramArguments</key>\n\
         \t<array>\n\
         \t\t<string>{program}</string>\n\
         \t</array>\n\
         \t<key>RunAtLoad</key>\n\
         \t<true/>\n\
         \t<key>KeepAlive</key>\n\
         \t<false/>\n\
         \t<key>LimitLoadToSessionType</key>\n\
         \t<string>Aqua</string>\n\
         \t<key>ProcessType</key>\n\
         \t<string>Interactive</string>\n\
         </dict>\n\
         </plist>\n",
        label = xml_escape(label),
        program = xml_escape(&program.to_string_lossy()),
    )
}
