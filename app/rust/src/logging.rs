//! Куда идут логи ядра и сервиса (`tracing`).
//!
//! - Android: в logcat, тег `braid`, от уровня `info`. Без подписчика все
//!   события ядра, движков и сервиса теряются (в logcat попадали бы только
//!   `log`-сообщения самой flutter_rust_bridge).
//! - macOS и остальные ОС — как было: подписчик не ставится, FRB сама
//!   направляет `log` в консоль ОС.
//!
//! Что пишется в журнал, решает ядро: ключей, токенов, хешей VK, доменов и
//! адресов пользователя в логах нет (правило 4), а уровни `debug`/`trace`
//! сюда не доходят — logcat не должен забиваться шумом HTTP и TLS.

#[cfg(any(target_os = "android", test))]
use tracing::Subscriber;
#[cfg(any(target_os = "android", test))]
use tracing_subscriber::filter::LevelFilter;
#[cfg(any(target_os = "android", test))]
use tracing_subscriber::layer::SubscriberExt;
#[cfg(any(target_os = "android", test))]
use tracing_subscriber::{Layer, Registry};

/// Тег записей в logcat (системный лимит — 23 байта).
#[cfg(target_os = "android")]
const LOGCAT_TAG: &str = "braid";

/// Самый подробный уровень, который уходит в системный журнал.
#[cfg(any(target_os = "android", test))]
const MAX_LEVEL: LevelFilter = LevelFilter::INFO;

/// Подписчик процесса: события не подробнее [`MAX_LEVEL`] идут на `output`.
#[cfg(any(target_os = "android", test))]
fn subscriber<L>(output: L) -> impl Subscriber + Send + Sync
where
    L: Layer<Registry> + Send + Sync + 'static,
{
    Registry::default().with(output).with(MAX_LEVEL)
}

/// Поставить подписчика процесса. Безопасно звать повторно: если подписчик
/// уже стоит (сервис поднял ядро раньше окна), второй вызов ничего не меняет.
pub fn init() {
    #[cfg(target_os = "android")]
    if let Ok(layer) = tracing_android::layer(LOGCAT_TAG) {
        // Занятый глобальный подписчик — не ошибка: логи уже идут куда надо.
        let _ = tracing::subscriber::set_global_default(subscriber(layer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::Event;
    use tracing_subscriber::layer::Context;

    /// Слой-«logcat»: запоминает уровень и текст каждого события.
    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<String>>>);

    struct Message(String);

    impl Visit for Message {
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            if field.name() == "message" {
                self.0 = format!("{value:?}");
            }
        }
    }

    impl<S: Subscriber> Layer<S> for Capture {
        fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
            let mut message = Message(String::new());
            event.record(&mut message);
            self.0
                .lock()
                .unwrap()
                .push(format!("{} {}", event.metadata().level(), message.0));
        }
    }

    #[test]
    fn only_info_and_above_reach_the_output() {
        let capture = Capture::default();
        tracing::subscriber::with_default(subscriber(capture.clone()), || {
            tracing::trace!("trace");
            tracing::debug!("debug");
            tracing::info!("info");
            tracing::warn!("warn");
            tracing::error!("error");
        });
        assert_eq!(
            *capture.0.lock().unwrap(),
            ["INFO info", "WARN warn", "ERROR error"]
        );
    }

    #[test]
    fn init_off_android_installs_nothing_and_is_repeatable() {
        // Хост — не Android: подписчик не ставится, повторный вызов безвреден.
        init();
        init();
        tracing::dispatcher::get_default(|dispatch| {
            assert!(dispatch.is::<tracing::subscriber::NoSubscriber>());
        });
    }
}
