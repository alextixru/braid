//! Журнал и телеметрия админа — тикет 03.
//!
//! `Journal { log(LogEvent), rotate(), uploader: Option<TelemetryUploader> }`:
//! JSONL (`braid_proto::LogRecord`) в каталоге `Storage::dir()`, ротация 30 дней,
//! отправка пакетами `POST /telemetry` при переключении и раз в час с накоплением
//! и повтором. Без `user.admin` в подписке — ни файла, ни отправки, только кольцевой
//! буфер ошибок в памяти (G02.3).
