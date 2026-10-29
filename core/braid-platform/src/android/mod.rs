//! Android: JNI-мост к `org.braid.vpn.BraidVpnService` — реализация в тикете 09.
//!
//! Kotlin → Rust: `nativeInit(storageDir)`, `nativeOnTunFd(fd, mtu)`,
//! `nativeNetworkChanged(id, kind)`, `nativeOriginalDns(String[])`, `nativeStop()`.
//! Rust → Kotlin: `protect(fd)`, `requestEstablish(json: TunConfig)`, `notify(json)`,
//! `updateNotification(json)`. `update_routes`/`update_apps` → re-establish →
//! новый fd через `Core::on_tun_fd`. Ядро одно на процесс (правило 10).
