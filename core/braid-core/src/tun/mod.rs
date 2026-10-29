//! Ввод-вывод tun поверх выданного fd — тикет 02.
//!
//! `TunIo { read(&mut buf) -> Pkt, write(pkt), swap_fd(fd) }`: на macOS снимает
//! и ставит 4-байтный AF-заголовок (`TunHandle::af_header`), на Linux/Android —
//! голый IP; `swap_fd` — бесшовная подмена при Android re-establish.
//! Watchdog (ист. 28): нет чтения 30 с при живых пробах → перезапуск сессии.
