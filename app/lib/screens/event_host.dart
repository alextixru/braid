import 'dart:async';

import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/ui_requests.dart';
import 'package:braid/screens/dialogs.dart';
import 'package:flutter/material.dart';

/// Показывает то, о чём просит контроллер: диалоги подтверждения (по одному,
/// в порядке запросов) и короткие сообщения внизу экрана. Стоит над
/// навигатором, поэтому работает поверх любого экрана.
class EventHost extends StatefulWidget {
  const EventHost({
    super.key,
    required this.controller,
    required this.navigatorKey,
    required this.messengerKey,
    required this.child,
  });

  final BraidController controller;
  final GlobalKey<NavigatorState> navigatorKey;
  final GlobalKey<ScaffoldMessengerState> messengerKey;
  final Widget child;

  @override
  State<EventHost> createState() => _EventHostState();
}

class _EventHostState extends State<EventHost> {
  final _queue = <UiRequest<Object?>>[];
  bool _showing = false;
  late StreamSubscription<UiRequest<Object?>> _requests;
  late StreamSubscription<Notice> _notices;

  @override
  void initState() {
    super.initState();
    _listen();
  }

  void _listen() {
    _requests = widget.controller.requests.listen((request) {
      _queue.add(request);
      unawaited(_showQueued());
    });
    _notices = widget.controller.notices.listen(_showNotice);
  }

  @override
  void didUpdateWidget(EventHost oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller != widget.controller) {
      unawaited(_requests.cancel());
      unawaited(_notices.cancel());
      _listen();
    }
  }

  @override
  void dispose() {
    unawaited(_requests.cancel());
    unawaited(_notices.cancel());
    // Незавершённые вопросы не должны висеть: отвечаем «нет».
    for (final request in _queue) {
      _decline(request);
    }
    super.dispose();
  }

  void _showNotice(Notice notice) {
    final messenger = widget.messengerKey.currentState;
    if (messenger == null) return;
    messenger
      ..hideCurrentSnackBar()
      ..showSnackBar(SnackBar(content: Text(notice.message)));
  }

  Future<void> _showQueued() async {
    if (_showing) return;
    _showing = true;
    try {
      while (_queue.isNotEmpty && mounted) {
        final request = _queue.removeAt(0);
        var context = widget.navigatorKey.currentContext;
        if (context == null) {
          // Навигатор ещё не построен — ждём кадр.
          await WidgetsBinding.instance.endOfFrame;
          context = widget.navigatorKey.currentContext;
        }
        if (context == null || !context.mounted) {
          _decline(request);
          continue;
        }
        await _ask(context, request);
      }
    } finally {
      _showing = false;
    }
  }

  Future<void> _ask(BuildContext context, UiRequest<Object?> request) async {
    switch (request) {
      case ConfirmReplace():
        request.complete(await showConfirmReplace(context, request));
      case ConfirmDomain():
        request.complete(await showConfirmDomain(context, request));
      case OfferSudoers():
        request.complete(await showSudoersOffer(context, request));
    }
  }

  void _decline(UiRequest<Object?> request) {
    switch (request) {
      case ConfirmReplace():
        request.complete(false);
      case ConfirmDomain():
        request.complete(false);
      case OfferSudoers():
        request.complete(SudoersAnswer.later);
    }
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
