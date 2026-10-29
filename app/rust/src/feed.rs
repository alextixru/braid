//! Лента состояний для Dart (`watch_state`). Один поток на подписчика:
//! ждёт, пока появится ядро (на Android его строит сервис, и оно позже окна),
//! шлёт кадр «не готово» один раз, затем текущий снимок и каждое изменение.
//! Поток выходит, когда приёмник закрыт (`emit` вернул `false`) или ядро
//! исчезло.

use crate::wire;
use braid_core::{Core, CoreState};
use std::time::Duration;

/// Как часто проверять, не появилось ли ядро.
pub const CORE_POLL: Duration = Duration::from_millis(250);

/// Гонит кадры в `emit` до закрытия приёмника. `lookup` отдаёт ядро, когда оно
/// есть; `emit` возвращает `false`, если слушателя больше нет.
pub fn pump<L, E>(lookup: L, poll: Duration, mut emit: E)
where
    L: Fn() -> Option<&'static Core>,
    E: FnMut(String) -> bool,
{
    let core = match lookup() {
        Some(core) => core,
        None => {
            if !emit(wire::frame(false, &CoreState::default())) {
                return;
            }
            // Следующий кадр — только когда ядро появится: «не готово»
            // повторять незачем.
            loop {
                std::thread::sleep(poll);
                if let Some(core) = lookup() {
                    break core;
                }
            }
        }
    };
    follow(core, emit);
}

fn follow<E: FnMut(String) -> bool>(core: &'static Core, mut emit: E) {
    let Ok(runtime) = tokio::runtime::Builder::new_current_thread().build() else {
        return;
    };
    let mut receiver = core.subscribe();
    let first = wire::frame(true, &receiver.borrow_and_update());
    if !emit(first) {
        return;
    }
    while runtime.block_on(receiver.changed()).is_ok() {
        let frame = wire::frame(true, &receiver.borrow_and_update());
        if !emit(frame) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::core_with_subscription;
    use serde_json::Value;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::sync::OnceLock;

    const FAST: Duration = Duration::from_millis(5);

    fn leak(core: braid_core::Core) -> &'static Core {
        Box::leak(Box::new(core))
    }

    fn next(rx: &mpsc::Receiver<String>) -> Value {
        let text = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("кадр не пришёл за 5 с");
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn a_ready_core_streams_its_snapshot_and_every_change() {
        let core = leak(core_with_subscription().core);
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            pump(
                move || Some(core),
                FAST,
                move |frame| tx.send(frame).is_ok(),
            )
        });
        let first = next(&rx);
        assert_eq!(first["ready"], Value::Bool(true));
        assert_eq!(first["state"]["subscription"]["host_id"], "frankfurt");

        let mut changed = core.state();
        changed.admin = false;
        core.publish(changed);
        let second = next(&rx);
        assert_eq!(second["ready"], Value::Bool(true));
        assert_eq!(second["state"]["admin"], Value::Bool(false));

        // Слушатель ушёл: следующая отправка не удалась — поток обязан выйти.
        drop(rx);
        let mut again = core.state();
        again.admin = true;
        core.publish(again);
        worker.join().expect("поток ленты завершился");
    }

    #[test]
    fn before_the_core_exists_the_feed_says_not_ready_once_then_follows_it() {
        let cell: Arc<OnceLock<&'static Core>> = Arc::new(OnceLock::new());
        let lookup_cell = cell.clone();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            pump(
                move || lookup_cell.get().copied(),
                FAST,
                move |frame| tx.send(frame).is_ok(),
            )
        });
        let waiting = next(&rx);
        assert_eq!(waiting["ready"], Value::Bool(false));
        // Пока ядра нет, «не готово» не повторяется.
        assert!(rx.recv_timeout(Duration::from_millis(60)).is_err());

        let core = leak(core_with_subscription().core);
        cell.set(core).ok().unwrap();
        let ready = next(&rx);
        assert_eq!(ready["ready"], Value::Bool(true));
        assert_eq!(ready["state"]["subscription"]["host_id"], "frankfurt");

        drop(rx);
        let mut changed = core.state();
        changed.admin = false;
        core.publish(changed);
        worker.join().expect("поток ленты завершился");
    }

    #[test]
    fn a_listener_that_leaves_while_the_core_is_missing_stops_the_feed() {
        let (tx, rx) = mpsc::channel::<String>();
        drop(rx);
        let worker =
            std::thread::spawn(move || pump(|| None, FAST, move |frame| tx.send(frame).is_ok()));
        worker.join().expect("поток ленты завершился");
    }
}
