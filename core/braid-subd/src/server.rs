//! Цикл приёма: TCP → TLS (rustls) → HTTP/1.1 (hyper), по задаче на соединение.

use crate::config::Config;
use crate::service::App;
use crate::tls;
use anyhow::Context as _;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper_util::rt::{TokioIo, TokioTimer};
use rustls::ServerConfig;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

/// Сколько ждём TLS-рукопожатие и заголовки запроса от одного соединения.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
const HEADER_TIMEOUT: Duration = Duration::from_secs(15);

/// Запуск по конфигу: токен, сертификат, каталоги, слушающий сокет.
pub async fn serve(config: Config) -> anyhow::Result<()> {
    let app = App::load(&config)?;
    let tls = tls::server_config(&config.cert, &config.key)?;
    let listener = TcpListener::bind(config.listen)
        .await
        .with_context(|| format!("не удалось занять {}", config.listen))?;
    match tls::key_pin(&config.key) {
        Ok(pin) => tracing::info!(listen = %config.listen, pin = %pin, "braid-subd слушает"),
        Err(e) => {
            tracing::warn!(listen = %config.listen, error = %e, "braid-subd слушает; пин не прочитан")
        }
    }
    serve_listener(listener, tls, app).await
}

/// Обслуживать уже открытый сокет — так сервер поднимается в тестах на `127.0.0.1:0`.
pub async fn serve_listener(
    listener: TcpListener,
    tls: Arc<ServerConfig>,
    app: Arc<App>,
) -> anyhow::Result<()> {
    let acceptor = TlsAcceptor::from(tls);
    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                // Кончились дескрипторы или сброс на accept — не падаем, ждём.
                tracing::warn!(error = %e, "accept");
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
        };
        let acceptor = acceptor.clone();
        let app = app.clone();
        tokio::spawn(async move {
            let tls = match tokio::time::timeout(HANDSHAKE_TIMEOUT, acceptor.accept(stream)).await {
                Ok(Ok(tls)) => tls,
                Ok(Err(e)) => {
                    tracing::debug!(error = %e, "TLS-рукопожатие не удалось");
                    return;
                }
                Err(_) => {
                    tracing::debug!("TLS-рукопожатие: тайм-аут");
                    return;
                }
            };
            let service = service_fn(move |req| app.clone().handle(req));
            let conn = http1::Builder::new()
                .timer(TokioTimer::new())
                .header_read_timeout(HEADER_TIMEOUT)
                .keep_alive(true)
                .serve_connection(TokioIo::new(tls), service);
            if let Err(e) = conn.await {
                tracing::debug!(error = %e, "соединение закрыто с ошибкой");
            }
        });
    }
}
