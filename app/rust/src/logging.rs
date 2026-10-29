//! Куда идут логи ядра и сервиса (`tracing`).
//!
//! - Android: в logcat, тег `braid`, от уровня `info`. Подписчика ставит
//!   `braid_runtime::android::init_logcat` — из `JNI_OnLoad`, когда сервис
//!   грузит библиотеку (`System.loadLibrary`): так логи старта по boot не
//!   теряются. Если первым библиотеку открыл Dart (`RustLib.init()` —
//!   `dlopen`, `JNI_OnLoad` не зовётся), подписчика тем же входом ставит
//!   [`init`]; кто пришёл вторым, ничего не меняет — второго подписчика нет.
//! - macOS и остальные ОС — как было: подписчик не ставится, FRB сама
//!   направляет `log` в консоль ОС.
//!
//! Что пишется в журнал, решает ядро: ключей, токенов, хешей VK, доменов и
//! адресов пользователя в логах нет (правило 4), а уровни `debug`/`trace`
//! до logcat не доходят — политика уровней живёт в `braid_runtime::android`.

/// Поставить подписчика процесса. Безопасно звать повторно и после
/// `JNI_OnLoad`: занятый глобальный подписчик — не ошибка, паники нет.
pub fn init() {
    #[cfg(target_os = "android")]
    {
        braid_runtime::android::init_logcat();
    }
}

#[cfg(test)]
mod tests {
    use super::init;
    use braid_runtime::android::logcat_subscriber;
    use std::sync::{Arc, Mutex};
    use tracing::field::{Field, Visit};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::Layer;

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

    /// Политика уровней — та же, что в logcat: подписчик берётся из runtime.
    #[test]
    fn only_info_and_above_reach_the_output() {
        let capture = Capture::default();
        tracing::subscriber::with_default(logcat_subscriber(capture.clone()), || {
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
