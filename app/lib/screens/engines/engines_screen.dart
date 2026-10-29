/// Экран «Движки» (ист. 14–17, 22, 30, 31, 74): каждый движок с живостью,
/// закрепить или вернуть «Авто», «Проверить связь»; для CSQTT — хеши звонков,
/// вход через VK и капча.
library;

import 'dart:async';

import 'package:braid/controller/braid_scope.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/screens/engines/csqtt_section.dart';
import 'package:braid/screens/engines/engine_card.dart';
import 'package:braid/screens/engines/engine_services.dart';
import 'package:braid/screens/engines/probe_section.dart';
import 'package:braid/tray/notification_permission.dart';
import 'package:braid/util/format.dart';
import 'package:braid/widgets/banner_card.dart';
import 'package:braid/widgets/page_body.dart';
import 'package:flutter/material.dart';

/// Подключается как `AppExtensions.engines`.
Widget buildEnginesScreen(BuildContext context) => const EnginesScreen();

class EnginesScreen extends StatefulWidget {
  const EnginesScreen({super.key});

  @override
  State<EnginesScreen> createState() => _EnginesScreenState();
}

class _EnginesScreenState extends State<EnginesScreen> {
  EngineServices? _services;

  @override
  void didChangeDependencies() {
    super.didChangeDependencies();
    if (_services != null) return;
    final services = EngineServices.of(BraidScope.read(context));
    _services = services;
    // Хеши и вход читаются при запуске ядра; здесь — на случай, если экран
    // открыт раньше (повторный вызов ничего не делает).
    unawaited(services.csqtt.load());
    // Разрешение могли выдать в системных настройках, пока экрана не было.
    unawaited(services.notifications?.recheck());
  }

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final services = _services!;
    final state = controller.state;
    final engines = state.engines;
    final hasCsqtt = engines.any((engine) => engine.kind == 'csqtt');
    final pinned = engines.where((engine) => engine.pinned).firstOrNull;
    final notifications = services.notifications;

    return Scaffold(
      appBar: AppBar(title: const Text('Движки')),
      body: PageBody(
        children: [
          if (notifications != null)
            ListenableBuilder(
              listenable: notifications,
              builder: (context, _) => notifications.missing
                  ? Padding(
                      padding: const EdgeInsets.only(bottom: 12),
                      child: BannerCard(
                        key: const Key('notifications-hint'),
                        icon: Icons.notifications_off_outlined,
                        title: 'Уведомления выключены',
                        subtitle: kNotificationHint,
                        actions: [
                          TextButton(
                            key: const Key('notifications-open-settings'),
                            onPressed: notifications.openSettings,
                            child: const Text('Открыть настройки'),
                          ),
                        ],
                      ),
                    )
                  : const SizedBox.shrink(),
            ),
          if (engines.isEmpty)
            const Text('В подписке нет движков')
          else ...[
            _ModeSummary(pinned: pinned, engines: engines),
            const SizedBox(height: 12),
            for (final engine in engines) ...[
              EngineCard(
                engine: engine,
                all: engines,
                active: state.active == engine.id,
                onTogglePin: () => controller.setEngineMode(
                  engine.pinned
                      ? const EngineMode.auto()
                      : EngineMode.pinned(engine.id),
                ),
              ),
              const SizedBox(height: 8),
            ],
          ],
          const ProbeSection(),
          if (hasCsqtt) const CsqttSection(),
        ],
      ),
    );
  }
}

/// Как выбирается движок: по приоритету («Авто») или закреплён вручную.
class _ModeSummary extends StatelessWidget {
  const _ModeSummary({required this.pinned, required this.engines});

  final EngineView? pinned;
  final List<EngineView> engines;

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.read(context);
    final pinned = this.pinned;
    return Card(
      child: ListTile(
        leading: const Icon(Icons.tune),
        title: Text(
          pinned == null
              ? 'Авто — движок выбирается по приоритету'
              : 'Закреплён вручную: ${engineLabel(pinned, engines)}',
        ),
        subtitle: Text(
          pinned == null
              ? 'Запасной движок подключён заранее и берёт трафик, если '
                    'активный отказал.'
              : 'Автопереключения нет, даже если движок недоступен.',
        ),
        trailing: pinned == null
            ? null
            : TextButton(
                key: const Key('engines-auto'),
                onPressed: () =>
                    controller.setEngineMode(const EngineMode.auto()),
                child: const Text('Вернуть Авто'),
              ),
      ),
    );
  }
}
