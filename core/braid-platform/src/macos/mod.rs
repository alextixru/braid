//! macOS: клиент root-помощника `braid-helper` — реализация в тикете 08.
//!
//! Протокол помощника — JSON-строки по UDS (`interfaces.md`): `conf`, `routes`,
//! `exclude`, `cleanup`, `down`; ответы `{"name", fd по SCM_RIGHTS}`,
//! `{"applied", "error"}`, `{"log"}`. Проверка собеседника — `LOCAL_PEERCRED`.
//! `open_tun` отдаёт `af_header = true`. Запуск: `sudo -n` при
//! `/etc/sudoers.d/braid`, иначе `osascript … with administrator privileges`.
