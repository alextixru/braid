import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/models/commands.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/services/vpn_channel.dart';
import 'package:braid/widgets/section_header.dart';
import 'package:flutter/material.dart';

/// Приложения в правилах (только Android, ист. 42, 43): режим «по спискам»
/// с исключёнными приложениями (банки — всегда напрямую) или «по приложениям»
/// — выбранные целиком в VPN. Смена режима пересоздаёт интерфейс, поэтому
/// при подключении просим подтвердить короткий разрыв.
class AppsSection extends StatefulWidget {
  const AppsSection({super.key});

  @override
  State<AppsSection> createState() => _AppsSectionState();
}

class _AppsSectionState extends State<AppsSection> {
  static const _lists = 'lists';
  static const _apps = 'apps';

  Future<List<InstalledApp>?>? _installed;

  Future<List<InstalledApp>?> _loadApps(BraidController controller) =>
      _installed ??= controller.vpn.installedApps();

  Future<void> _selectMode(BraidController controller, String mode) async {
    final current = controller.state.rules.apps;
    final alreadyApps = current is AppRulesAllowed;
    if ((mode == _apps) == alreadyApps) return;
    final session = controller.state.session;
    final connected =
        session is SessionConnected || session is SessionConnecting;
    if (connected && !await _confirmBreak(context)) return;
    final error = await controller.applyRules(
      RulesChange.setApps(
        mode == _apps ? const AppRulesAllowed([]) : const AppRulesAll(),
      ),
    );
    if (error != null) controller.showNotice(error);
  }

  Future<void> _addApp(BraidController controller) async {
    final installed = await _loadApps(controller);
    if (!mounted) return;
    final current = controller.state.rules.apps;
    final taken = current.packages.toSet();
    final package = await showAppPicker(
      context,
      installed?.where((app) => !taken.contains(app.package)).toList(),
    );
    if (package == null || taken.contains(package)) return;
    await _apply(controller, [...current.packages, package]);
  }

  Future<void> _removeApp(BraidController controller, String package) {
    final current = controller.state.rules.apps;
    return _apply(controller, [
      for (final p in current.packages)
        if (p != package) p,
    ]);
  }

  Future<void> _apply(BraidController controller, List<String> packages) async {
    final apps = controller.state.rules.apps;
    final AppRules next = apps is AppRulesAllowed
        ? AppRulesAllowed(packages)
        : packages.isEmpty
        ? const AppRulesAll()
        : AppRulesExcluded(packages);
    final error = await controller.applyRules(RulesChange.setApps(next));
    if (error != null) controller.showNotice(error);
  }

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final apps = controller.state.rules.apps;
    final byApps = apps is AppRulesAllowed;
    final theme = Theme.of(context);
    return FutureBuilder<List<InstalledApp>?>(
      future: _loadApps(controller),
      builder: (context, snapshot) {
        final labels = {
          for (final app in snapshot.data ?? const <InstalledApp>[])
            app.package: app.label,
        };
        return Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            const SectionHeader(
              'Приложения',
              subtitle: 'Смена режима пересоздаёт VPN-интерфейс — связь на мгновение пропадёт.',
            ),
            SizedBox(
              width: double.infinity,
              child: SegmentedButton<String>(
                key: const Key('apps-mode'),
                showSelectedIcon: false,
                segments: const [
                  ButtonSegment(value: _lists, label: Text('По спискам')),
                  ButtonSegment(value: _apps, label: Text('По приложениям')),
                ],
                selected: {byApps ? _apps : _lists},
                onSelectionChanged: (selection) =>
                    _selectMode(controller, selection.first),
              ),
            ),
            const SizedBox(height: 8),
            Text(
              byApps
                  ? 'Выбранные приложения целиком идут через VPN, остальные — мимо него.'
                  : 'Через VPN идут домены и подсети из наборов. Исключённые приложения '
                        'идут напрямую даже на домены из списков.',
              style: theme.textTheme.bodySmall?.copyWith(
                color: theme.colorScheme.onSurfaceVariant,
              ),
            ),
            const SizedBox(height: 12),
            Text(
              byApps
                  ? 'Приложения в VPN'
                  : 'Исключённые приложения (всегда напрямую)',
              style: theme.textTheme.labelLarge,
            ),
            if (byApps && apps.packages.isEmpty)
              Padding(
                padding: const EdgeInsets.only(top: 4),
                child: Text(
                  'Пока ни одно приложение не выбрано — всё работает по спискам, как в обычном режиме.',
                  style: theme.textTheme.bodySmall?.copyWith(
                    color: theme.colorScheme.error,
                  ),
                ),
              ),
            if (apps.packages.isNotEmpty)
              Card(
                child: Column(
                  children: [
                    for (final package in apps.packages)
                      ListTile(
                        key: Key('app-$package'),
                        leading: const Icon(Icons.apps),
                        title: Text(labels[package] ?? package),
                        subtitle: labels[package] == null
                            ? null
                            : Text(package),
                        trailing: IconButton(
                          key: Key('app-remove-$package'),
                          tooltip: 'Убрать',
                          icon: const Icon(Icons.delete_outline),
                          onPressed: () => _removeApp(controller, package),
                        ),
                      ),
                  ],
                ),
              ),
            const SizedBox(height: 8),
            Align(
              alignment: Alignment.centerLeft,
              child: OutlinedButton.icon(
                key: const Key('app-add'),
                onPressed: () => _addApp(controller),
                icon: const Icon(Icons.add),
                label: const Text('Добавить приложение'),
              ),
            ),
          ],
        );
      },
    );
  }
}

Future<bool> _confirmBreak(BuildContext context) async {
  final answer = await showDialog<bool>(
    context: context,
    builder: (context) => AlertDialog(
      title: const Text('Сменить режим?'),
      content: const Text(
        'Смена режима пересоздаёт VPN-интерфейс: связь на мгновение разорвётся, '
        'открытые соединения придётся переустановить.',
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context, false),
          child: const Text('Отмена'),
        ),
        FilledButton(
          key: const Key('apps-mode-confirm'),
          onPressed: () => Navigator.pop(context, true),
          child: const Text('Сменить'),
        ),
      ],
    ),
  );
  return answer ?? false;
}

final _packageName = RegExp(
  r'^[A-Za-z][A-Za-z0-9_]*(\.[A-Za-z][A-Za-z0-9_]*)+$',
);

/// Выбрать приложение: из списка установленных или (если система списка не
/// даёт) по имени пакета.
Future<String?> showAppPicker(
  BuildContext context,
  List<InstalledApp>? installed,
) {
  return showDialog<String>(
    context: context,
    builder: (context) => installed == null || installed.isEmpty
        ? const _PackageNameDialog()
        : _InstalledAppsDialog(apps: installed),
  );
}

class _InstalledAppsDialog extends StatefulWidget {
  const _InstalledAppsDialog({required this.apps});

  final List<InstalledApp> apps;

  @override
  State<_InstalledAppsDialog> createState() => _InstalledAppsDialogState();
}

class _InstalledAppsDialogState extends State<_InstalledAppsDialog> {
  String _query = '';

  @override
  Widget build(BuildContext context) {
    final query = _query.trim().toLowerCase();
    final shown = [
      for (final app in widget.apps)
        if (query.isEmpty ||
            app.label.toLowerCase().contains(query) ||
            app.package.toLowerCase().contains(query))
          app,
    ];
    return AlertDialog(
      title: const Text('Выбери приложение'),
      content: SizedBox(
        width: 400,
        height: 420,
        child: Column(
          children: [
            TextField(
              decoration: const InputDecoration(
                prefixIcon: Icon(Icons.search),
                hintText: 'Поиск',
              ),
              onChanged: (value) => setState(() => _query = value),
            ),
            const SizedBox(height: 8),
            Expanded(
              child: ListView(
                children: [
                  for (final app in shown)
                    ListTile(
                      title: Text(app.label),
                      subtitle: Text(app.package),
                      onTap: () => Navigator.pop(context, app.package),
                    ),
                ],
              ),
            ),
          ],
        ),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Отмена'),
        ),
      ],
    );
  }
}

class _PackageNameDialog extends StatefulWidget {
  const _PackageNameDialog();

  @override
  State<_PackageNameDialog> createState() => _PackageNameDialogState();
}

class _PackageNameDialogState extends State<_PackageNameDialog> {
  final _field = TextEditingController();
  String? _error;

  @override
  void dispose() {
    _field.dispose();
    super.dispose();
  }

  void _submit() {
    final text = _field.text.trim();
    if (!_packageName.hasMatch(text)) {
      setState(() => _error = 'Имя пакета выглядит так: org.example.app');
      return;
    }
    Navigator.pop(context, text);
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: const Text('Имя пакета приложения'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          const Text('Система не отдала список приложений — введи имя пакета.'),
          const SizedBox(height: 12),
          TextField(
            key: const Key('app-package-field'),
            controller: _field,
            autocorrect: false,
            enableSuggestions: false,
            decoration: InputDecoration(
              labelText: 'Пакет',
              hintText: 'org.example.app',
              errorText: _error,
            ),
            onChanged: (_) => setState(() => _error = null),
            onSubmitted: (_) => _submit(),
          ),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.pop(context),
          child: const Text('Отмена'),
        ),
        FilledButton(
          key: const Key('app-package-add'),
          onPressed: _submit,
          child: const Text('Добавить'),
        ),
      ],
    );
  }
}
