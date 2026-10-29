import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

/// Название приложения — одной константой (спецификация, «Открытые места»).
const String appName = 'Braid';

/// Корень приложения: Material 3, нейтральная палитра, русская локаль.
/// Версия ядра приходит снаружи, чтобы виджеты не зависели от моста Rust.
class BraidApp extends StatelessWidget {
  const BraidApp({super.key, required this.coreVersion});

  final String coreVersion;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: appName,
      theme: ThemeData(
        useMaterial3: true,
        colorScheme: ColorScheme.fromSeed(seedColor: Colors.blueGrey),
      ),
      darkTheme: ThemeData(
        useMaterial3: true,
        colorScheme: ColorScheme.fromSeed(
          seedColor: Colors.blueGrey,
          brightness: Brightness.dark,
        ),
      ),
      locale: const Locale('ru'),
      supportedLocales: const [Locale('ru')],
      localizationsDelegates: const [
        GlobalMaterialLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
      ],
      home: HomeScreen(coreVersion: coreVersion),
    );
  }
}

/// Экран-заглушка до тикета 10: состояние «подписки нет», кнопка недоступна.
class HomeScreen extends StatelessWidget {
  const HomeScreen({super.key, required this.coreVersion});

  final String coreVersion;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      appBar: AppBar(title: const Text(appName)),
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(24),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              Icon(Icons.vpn_lock_outlined, size: 64, color: theme.colorScheme.primary),
              const SizedBox(height: 16),
              Text('Подписки нет', style: theme.textTheme.titleLarge),
              const SizedBox(height: 8),
              const Text('Вставь ссылку или отсканируй QR'),
              const SizedBox(height: 24),
              const FilledButton(onPressed: null, child: Text('Подключить')),
              const SizedBox(height: 32),
              Text('Ядро $coreVersion', style: theme.textTheme.bodySmall),
            ],
          ),
        ),
      ),
    );
  }
}
