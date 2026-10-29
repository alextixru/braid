import 'dart:async';

import 'package:braid/app_extensions.dart';
import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/controller/connect_models.dart';
import 'package:braid/screens/event_host.dart';
import 'package:braid/screens/gate.dart';
import 'package:braid/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

/// Название приложения — одной константой (спецификация, «Открытые места»).
const String appName = 'Braid';

/// Корень приложения: Material 3, нейтральная палитра, русская локаль.
/// Всё остальное — в [controller]: он говорит с ядром через мост, экраны
/// только читают его. Запускает контроллер сам.
class BraidApp extends StatefulWidget {
  const BraidApp({
    super.key,
    required this.controller,
    this.extensions = kAppExtensions,
  });

  final BraidController controller;
  final AppExtensions extensions;

  @override
  State<BraidApp> createState() => _BraidAppState();
}

class _BraidAppState extends State<BraidApp> with WidgetsBindingObserver {
  final _navigatorKey = GlobalKey<NavigatorState>(
    debugLabel: 'braid-navigator',
  );
  final _messengerKey = GlobalKey<ScaffoldMessengerState>(
    debugLabel: 'braid-messenger',
  );
  bool _readyFired = false;

  @override
  void initState() {
    super.initState();
    WidgetsBinding.instance.addObserver(this);
    widget.controller.addListener(_onController);
    unawaited(widget.controller.start());
  }

  @override
  void didUpdateWidget(BraidApp oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (oldWidget.controller == widget.controller) return;
    // Тот же State, другой контроллер (горячая замена, тесты): отписываемся от
    // прежнего и запускаем новый, иначе он вечно «запускается».
    oldWidget.controller.removeListener(_onController);
    _readyFired = false;
    widget.controller.addListener(_onController);
    unawaited(widget.controller.start());
  }

  @override
  void dispose() {
    widget.controller.removeListener(_onController);
    WidgetsBinding.instance.removeObserver(this);
    super.dispose();
  }

  @override
  void didChangeAppLifecycleState(AppLifecycleState state) {
    if (state == AppLifecycleState.resumed) {
      unawaited(widget.controller.onAppResumed());
    }
  }

  void _onController() {
    if (_readyFired || widget.controller.phase != StartPhase.ready) return;
    _readyFired = true;
    unawaited(
      Future.sync(() => widget.extensions.onReady?.call(widget.controller)),
    );
  }

  @override
  Widget build(BuildContext context) {
    return BraidScope(
      controller: widget.controller,
      extensions: widget.extensions,
      navigatorKey: _navigatorKey,
      child: MaterialApp(
        title: appName,
        debugShowCheckedModeBanner: false,
        navigatorKey: _navigatorKey,
        scaffoldMessengerKey: _messengerKey,
        theme: buildTheme(Brightness.light),
        darkTheme: buildTheme(Brightness.dark),
        locale: const Locale('ru'),
        supportedLocales: const [Locale('ru')],
        localizationsDelegates: const [
          GlobalMaterialLocalizations.delegate,
          GlobalWidgetsLocalizations.delegate,
          GlobalCupertinoLocalizations.delegate,
        ],
        home: const Gate(),
        builder: (context, child) {
          final host = EventHost(
            controller: widget.controller,
            navigatorKey: _navigatorKey,
            messengerKey: _messengerKey,
            child: child ?? const SizedBox.shrink(),
          );
          return widget.extensions.wrapApp?.call(context, host) ?? host;
        },
      ),
    );
  }
}

/// Нативная библиотека ядра не загрузилась (нет файла, не подошла версия):
/// без неё приложение работать не может, но окно хотя бы объясняет причину.
class StartupFailedApp extends StatelessWidget {
  const StartupFailedApp({super.key, required this.message});

  final String message;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: appName,
      debugShowCheckedModeBanner: false,
      theme: buildTheme(Brightness.light),
      darkTheme: buildTheme(Brightness.dark),
      locale: const Locale('ru'),
      supportedLocales: const [Locale('ru')],
      localizationsDelegates: const [
        GlobalMaterialLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
      ],
      home: Builder(
        builder: (context) {
          final theme = Theme.of(context);
          return Scaffold(
            body: Center(
              child: ConstrainedBox(
                constraints: const BoxConstraints(maxWidth: 460),
                child: Padding(
                  padding: const EdgeInsets.all(24),
                  child: Column(
                    mainAxisSize: MainAxisSize.min,
                    children: [
                      Icon(
                        Icons.error_outline,
                        size: 56,
                        color: theme.colorScheme.error,
                      ),
                      const SizedBox(height: 16),
                      Text(
                        '$appName не запустился',
                        style: theme.textTheme.titleLarge,
                      ),
                      const SizedBox(height: 8),
                      SelectableText(message, textAlign: TextAlign.center),
                      const SizedBox(height: 16),
                      Text(
                        'Переустанови приложение. Если не помогло — сообщи тому, кто его выдал.',
                        textAlign: TextAlign.center,
                        style: theme.textTheme.bodySmall,
                      ),
                    ],
                  ),
                ),
              ),
            ),
          );
        },
      ),
    );
  }
}
