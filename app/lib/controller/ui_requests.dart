/// То, что контроллер просит у экрана: диалог с ответом или короткое
/// сообщение. Контроллер не знает про виджеты — он кладёт запрос в поток, а
/// `EventHost` показывает диалог и возвращает ответ. Так подтверждения
/// (замена подписки, добавление домена, правило sudoers) проверяются и без
/// экрана: тест отвечает на запрос сам.
library;

import 'dart:async';

import 'package:braid/bridge/braid_bridge.dart';

/// Запрос, на который нужен ответ [T].
sealed class UiRequest<T> {
  final Completer<T> _completer = Completer<T>();

  Future<T> get answer => _completer.future;

  bool get isAnswered => _completer.isCompleted;

  /// Ответить; повторный ответ игнорируется.
  void complete(T value) {
    if (!_completer.isCompleted) _completer.complete(value);
  }
}

/// Подтвердить замену подписки на ссылку другого хоста (ист. 76).
final class ConfirmReplace extends UiRequest<bool> {
  ConfirmReplace({required this.currentHost, required this.newHost});

  final String currentHost;
  final String newHost;
}

/// Подтвердить добавление домена из буфера обмена или share-меню (ист. 54).
final class ConfirmDomain extends UiRequest<bool> {
  ConfirmDomain(this.domain);

  final String domain;
}

enum SudoersAnswer { install, later }

/// Предложить правило sudoers: «Не спрашивать пароль в дальнейшем».
/// [update] — правило уже стоит, но привязано к прошлой сборке помощника.
final class OfferSudoers extends UiRequest<SudoersAnswer> {
  OfferSudoers({required this.plan, this.update = false});

  final SudoersPlan plan;
  final bool update;
}

/// Короткое сообщение внизу экрана.
class Notice {
  const Notice(this.message);

  final String message;
}
