/// Итоги операций контроллера для экранов, которым нужна ошибка на месте.
library;

/// Итог импорта ссылки (ввод, QR, deeplink).
sealed class ImportResult {
  const ImportResult();
}

/// Подписка загружена (в том числе после подтверждённой замены).
final class ImportDone extends ImportResult {
  const ImportDone();
}

/// Пользователь не подтвердил замену подписки.
final class ImportCancelled extends ImportResult {
  const ImportCancelled();
}

/// Ссылка не принята; [message] показывается на месте, ввод не теряется.
final class ImportFailed extends ImportResult {
  const ImportFailed(this.message);

  final String message;
}

/// Ответ на вопрос «подключаться в этой сети?».
enum AutoconnectAnswer {
  /// Подключиться сейчас, правило сети не менять.
  once,

  /// Подключиться и запомнить «подключаться» для сети.
  always,

  /// Запомнить «не подключаться» для сети.
  never,
}

enum StartPhase { starting, ready, failed }
