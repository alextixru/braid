import 'dart:async';

import 'package:braid/app.dart';
import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/screens/home/banner_stack.dart';
import 'package:braid/screens/home/connect_button.dart';
import 'package:braid/screens/home/engine_switch.dart';
import 'package:braid/screens/home/home_status.dart';
import 'package:braid/screens/home/mode_sheet.dart';
import 'package:braid/screens/rules/rules_screen.dart';
import 'package:braid/screens/settings/settings_screen.dart';
import 'package:braid/screens/subscription/subscription_screen.dart';
import 'package:braid/util/format.dart';
import 'package:braid/widgets/page_body.dart';
import 'package:flutter/material.dart';

/// Главный экран: кнопка и состояние, сеть, режим, движок, «+ Домен»,
/// плашки. Остальное — на второстепенных экранах (ист. 60).
class HomeScreen extends StatefulWidget {
  const HomeScreen({super.key});

  @override
  State<HomeScreen> createState() => _HomeScreenState();
}

class _HomeScreenState extends State<HomeScreen> {
  Timer? _clock;

  @override
  void initState() {
    super.initState();
    // Длительность подключения идёт сама, без новых кадров от ядра.
    _clock = Timer.periodic(const Duration(seconds: 30), (_) {
      if (mounted && BraidScope.read(context).connectedFor != null) {
        setState(() {});
      }
    });
  }

  @override
  void dispose() {
    _clock?.cancel();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final status = HomeStatus.of(controller);
    final theme = Theme.of(context);
    return Scaffold(
      appBar: AppBar(title: const Text(appName)),
      body: PageBody(
        children: [
          const SizedBox(height: 8),
          Center(
            child: ConnectButton(
              label: status.buttonLabel,
              tone: status.tone,
              // Идёт запуск сессии, а кадр «подключаюсь» ещё не пришёл: второй
              // тап не нужен.
              onPressed: controller.busy && status.tone != StatusTone.working
                  ? null
                  : controller.toggleConnection,
            ),
          ),
          const SizedBox(height: 16),
          Text(
            status.title,
            key: const Key('status-title'),
            textAlign: TextAlign.center,
            style: theme.textTheme.titleLarge,
          ),
          if (status.detail != null) ...[
            const SizedBox(height: 4),
            Text(
              status.detail!,
              key: const Key('status-detail'),
              textAlign: TextAlign.center,
              style: theme.textTheme.bodyMedium?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
          ],
          if (status.offerDisconnect)
            Center(
              child: TextButton(
                key: const Key('disconnect-button'),
                onPressed: controller.disconnect,
                child: const Text('Отключить'),
              ),
            ),
          const SizedBox(height: 24),
          // Плашки идут под состоянием: главная кнопка не уезжает вниз,
          // сколько бы их ни пришло.
          const BannerStack(),
          _InfoCard(controller: controller),
          const SizedBox(height: 12),
          FilledButton.tonal(
            key: const Key('add-domain-button'),
            onPressed: controller.addDomainFromClipboard,
            child: const Text('+ Домен'),
          ),
          const SizedBox(height: 4),
          Text(
            'Берёт домен или ссылку из буфера обмена',
            textAlign: TextAlign.center,
            style: theme.textTheme.bodySmall?.copyWith(
              color: theme.colorScheme.onSurfaceVariant,
            ),
          ),
          const SizedBox(height: 24),
          _NavigationCard(controller: controller),
        ],
      ),
    );
  }
}

class _InfoCard extends StatelessWidget {
  const _InfoCard({required this.controller});

  final BraidController controller;

  @override
  Widget build(BuildContext context) {
    final state = controller.state;
    final modeId = state.rules.mode;
    final modeName = modeId == null
        ? 'Своё'
        : controller.catalog.modeById(modeId)?.name ?? modeId;
    return Card(
      child: Column(
        children: [
          ListTile(
            key: const Key('network-row'),
            leading: Icon(
              state.network != null && state.network!.startsWith('wifi:')
                  ? Icons.wifi
                  : Icons.public,
            ),
            title: const Text('Сеть'),
            subtitle: Text(
              state.network == null
                  ? 'не определена'
                  : networkLabel(state.network!),
            ),
          ),
          const Divider(height: 1),
          ListTile(
            key: const Key('mode-row'),
            leading: const Icon(Icons.tune),
            title: const Text('Режим'),
            subtitle: Text(modeName),
            trailing: const Icon(Icons.chevron_right),
            onTap: () => showModeSheet(context, controller),
          ),
          if (state.engines.isNotEmpty) ...[
            const Divider(height: 1),
            const Padding(
              padding: EdgeInsets.fromLTRB(16, 12, 16, 12),
              child: Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [Text('Движок'), SizedBox(height: 8), EngineSwitch()],
              ),
            ),
          ],
        ],
      ),
    );
  }
}

class _NavigationCard extends StatelessWidget {
  const _NavigationCard({required this.controller});

  final BraidController controller;

  @override
  Widget build(BuildContext context) {
    final extensions = BraidScope.extensionsOf(context);
    final state = controller.state;
    final enabledSets = state.rules.sets.where((s) => s.enabled).length;
    final userRules =
        state.rules.userDomains.length + state.rules.userCidrs.length;
    final host =
        state.subscription.hostName ?? state.subscription.hostId ?? '—';

    void open(WidgetBuilder builder) =>
        Navigator.of(context).push(MaterialPageRoute<void>(builder: builder));

    final tiles = <Widget>[
      _NavTile(
        key: const Key('tile-rules'),
        icon: Icons.rule,
        title: 'Правила',
        subtitle:
            'Наборов включено: $enabledSets из ${state.rules.sets.length} · своих: $userRules',
        onTap: () => open((_) => const RulesScreen()),
      ),
      _NavTile(
        key: const Key('tile-subscription'),
        icon: Icons.cloud_sync_outlined,
        title: 'Подписка',
        subtitle: host,
        onTap: () => open((_) => const SubscriptionScreen()),
      ),
      if (extensions.engines != null)
        _NavTile(
          key: const Key('tile-engines'),
          icon: Icons.hub_outlined,
          title: 'Движки',
          onTap: () => open(extensions.engines!),
        ),
      _NavTile(
        key: const Key('tile-settings'),
        icon: Icons.settings_outlined,
        title: 'Настройки',
        onTap: () => open((_) => const SettingsScreen()),
      ),
      if (extensions.stats != null && state.admin)
        _NavTile(
          key: const Key('tile-stats'),
          icon: Icons.query_stats,
          title: 'Статистика',
          onTap: () => open(extensions.stats!),
        ),
      if (extensions.about != null)
        _NavTile(
          key: const Key('tile-about'),
          icon: Icons.info_outline,
          title: 'О программе',
          onTap: () => open(extensions.about!),
        ),
    ];
    return Card(
      child: Column(
        children: [
          for (var i = 0; i < tiles.length; i++) ...[
            if (i > 0) const Divider(height: 1),
            tiles[i],
          ],
        ],
      ),
    );
  }
}

class _NavTile extends StatelessWidget {
  const _NavTile({
    super.key,
    required this.icon,
    required this.title,
    this.subtitle,
    required this.onTap,
  });

  final IconData icon;
  final String title;
  final String? subtitle;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    return ListTile(
      leading: Icon(icon),
      title: Text(title),
      subtitle: subtitle == null ? null : Text(subtitle!),
      trailing: const Icon(Icons.chevron_right),
      onTap: onTap,
    );
  }
}
