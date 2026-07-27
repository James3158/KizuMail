#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|argument| argument == "--purge-user-data") {
        if let Err(error) = kizumail_lib::purge_user_data_cli() {
            eprintln!("KizuMail-Daten konnten nicht vollständig entfernt werden: {error}");
            std::process::exit(1);
        }
        return;
    }
    kizumail_lib::run();
}
