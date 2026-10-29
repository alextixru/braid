import 'package:braid/app.dart';
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/controller/connect_models.dart';
import 'package:braid/screens/home/home_screen.dart';
import 'package:braid/screens/onboarding/onboarding_screen.dart';
import 'package:flutter/material.dart';

/// Корневой экран: решает, что показать. Нет ядра — ожидание; нет подписки —
/// единственный экран «Вставь ссылку или отсканируй QR»; есть — главный.
class Gate extends StatelessWidget {
  const Gate({super.key});

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    if (controller.phase == StartPhase.failed) {
      return StartFailedScreen(
        message: controller.startError ?? 'Не удалось запустить ядро',
        onRetry: controller.start,
      );
    }
    if (controller.phase == StartPhase.starting || !controller.coreReady) {
      return SplashScreen(
        message: controller.platform.isAndroid
            ? 'Сервис запускается…'
            : 'Запускаю ядро…',
      );
    }
    if (!controller.state.hasSubscription) return const OnboardingScreen();
    return const HomeScreen();
  }
}

/// Ожидание ядра: на Android его строит сервис и оно появляется позже окна.
class SplashScreen extends StatelessWidget {
  const SplashScreen({super.key, required this.message});

  final String message;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            const CircularProgressIndicator(),
            const SizedBox(height: 24),
            Text(appName, style: Theme.of(context).textTheme.headlineSmall),
            const SizedBox(height: 8),
            Text(message, key: const Key('splash-message')),
          ],
        ),
      ),
    );
  }
}

/// Ядро не поднялось: причина и «Повторить».
class StartFailedScreen extends StatelessWidget {
  const StartFailedScreen({
    super.key,
    required this.message,
    required this.onRetry,
  });

  final String message;
  final Future<void> Function() onRetry;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return Scaffold(
      body: Center(
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 420),
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
                Text('Не удалось запустить', style: theme.textTheme.titleLarge),
                const SizedBox(height: 8),
                Text(
                  message,
                  key: const Key('start-error'),
                  textAlign: TextAlign.center,
                ),
                const SizedBox(height: 24),
                FilledButton(
                  key: const Key('start-retry'),
                  onPressed: onRetry,
                  child: const Text('Повторить'),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
