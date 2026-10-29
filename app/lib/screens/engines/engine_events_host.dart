/// Надстройка над всем приложением (`AppExtensions.wrapApp`): слушает ядро и
/// показывает то, что движок просит у пользователя поверх любого экрана, —
/// диалог капчи VK (ист. 16). Заодно следит за возвратом в приложение:
/// разрешение на уведомления могли выдать в системных настройках.
library;

import 'dart:async';

import 'package:braid/app.dart' show appName;
import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/screens/engines/engine_services.dart';
import 'package:braid/screens/engines/web_flow_dialog.dart';
import 'package:braid/services/captcha/captcha_coordinator.dart';
import 'package:flutter/widgets.dart';

/// Подключается как `AppExtensions.wrapApp`.
Widget wrapWithEngineEvents(BuildContext context, Widget app) =>
    EngineEventsHost(child: app);

class EngineEventsHost extends StatefulWidget {
  const EngineEventsHost({super.key, required this.child});

  final Widget child;

  @override
  State<EngineEventsHost> createState() => _EngineEventsHostState();
}

class _EngineEventsHostState extends State<EngineEventsHost>
    with WidgetsBindingObserver {
  BraidController? _controller;
  EngineServices? _services;
  bool _busy = false;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_controller != null) return;
    final controller = BraidScope.read(context);
    _controller = controller;
    _services = EngineServices.of(controller);
    controller.addListener(_onController);
    WidgetsBinding.instance.addObserver(this);
    // Капча могла прийти раньше, чем надстройка встала в дерево.
    WidgetsBinding.instance.addPostFrameCallback((_) => _onController());
  }

  @override
  void dispose() {
    _controller?.removeListener(_onController);
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      unawaited(_services?.notifications?.recheck());
    }
  }

  void _onController() {
    final services = _services;
    final controller = _controller;
    if (_busy || !mounted || services == null || controller == null) return;
    final request = services.captcha.nextRequest(controller.state);
    if (request == null) return;
    _busy = true;
    unawaited(
      _handle(
        services,
        controller,
        BraidScope.navigatorKeyOf(context),
        request,
      ),
    );
  }

  Future<void> _handle(
    EngineServices services,
    BraidController controller,
    GlobalKey<NavigatorState> navigatorKey,
    CaptchaRequest request,
  ) async {
    try {
      final flow = await services.captcha.begin(request);
      final result = await presentWebFlow<String>(
        navigatorKey: navigatorKey,
        flow: flow,
        title: 'Подтвердите, что вы не робот',
        hint:
            'Капча VK открыта в отдельном окне. Реши её там — '
            '$appName продолжит сам.',
        pageHint: 'Реши капчу VK — $appName продолжит сам.',
        dialogKey: const Key('captcha-dialog'),
      );
      final problem = await services.captcha.deliver(request, result);
      if (problem != null) controller.showNotice(problem);
    } finally {
      _busy = false;
      // Пока отвечали, ядро могло попросить следующую капчу.
      if (mounted) _onController();
    }
  }

  @override
  Widget build(BuildContext context) => widget.child;
}
