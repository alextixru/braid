import 'package:braid/app.dart' show appName;
import 'package:braid/controller/braid_controller.dart';
import 'package:braid/controller/braid_scope.dart';
import 'package:braid/controller/connect_models.dart';
import 'package:braid/models/core_state.dart';
import 'package:braid/screens/subscription/link_qr.dart';
import 'package:braid/util/format.dart';
import 'package:braid/widgets/page_body.dart';
import 'package:braid/widgets/section_header.dart';
import 'package:flutter/material.dart';

/// «Подписка»: хост, версия, дата, движки и наборы, «Обновить сейчас»,
/// ошибка последней попытки и давность, «Доступ отозван»; QR и копирование
/// ссылки; замена подписки другой ссылкой с подтверждением (ист. 35–38, 76).
class SubscriptionScreen extends StatefulWidget {
  const SubscriptionScreen({super.key});

  @override
  State<SubscriptionScreen> createState() => _SubscriptionScreenState();
}

class _SubscriptionScreenState extends State<SubscriptionScreen> {
  final _field = TextEditingController();
  String? _error;
  bool _busy = false;

  @override
  void dispose() {
    _field.dispose();
    super.dispose();
  }

  BraidController get _controller => BraidScope.read(context);

  void _setField(String text) {
    _field.value = TextEditingValue(
      text: text,
      selection: TextSelection.collapsed(offset: text.length),
    );
  }

  Future<void> _paste() async {
    final text = (await _controller.clipboard.readText())?.trim() ?? '';
    if (!mounted) return;
    if (text.isEmpty) {
      _controller.showNotice('В буфере пусто');
      return;
    }
    setState(() {
      _setField(text);
      _error = null;
    });
  }

  Future<void> _scan() async {
    final code = (await _controller.scanner.scan(context))?.trim();
    if (!mounted || code == null || code.isEmpty) return;
    setState(() => _setField(code));
    await _apply();
  }

  Future<void> _apply() async {
    if (_busy) return;
    setState(() {
      _busy = true;
      _error = null;
    });
    final result = await _controller.importLink(_field.text);
    if (!mounted) return;
    setState(() {
      _busy = false;
      switch (result) {
        case ImportDone():
          _field.clear();
        case ImportCancelled():
          break;
        case ImportFailed(:final message):
          _error = message;
      }
    });
    if (result is ImportDone) _controller.showNotice('Подписка загружена');
  }

  @override
  Widget build(BuildContext context) {
    final controller = BraidScope.of(context);
    final state = controller.state;
    final sub = state.subscription;
    final theme = Theme.of(context);
    final stale = state.banners.whereType<SubscriptionStaleBanner>();
    final now = controller.now();

    final updated = sub.updatedAt == null
        ? 'ещё не обновлялась'
        : formatWhen(sub.updatedAt!.toLocal(), now: now);
    final engines = state.engines
        .map((e) => engineLabel(e, state.engines))
        .join(', ');
    final enabled = state.rules.sets.where((s) => s.enabled).length;
    final sets = state.rules.sets.map((s) => s.name).join(', ');

    return Scaffold(
      appBar: AppBar(title: const Text('Подписка')),
      body: PageBody(
        children: [
          if (sub.revoked)
            Padding(
              padding: const EdgeInsets.only(bottom: 12),
              child: Card(
                key: const Key('sub-revoked'),
                color: theme.colorScheme.errorContainer,
                child: ListTile(
                  leading: Icon(
                    Icons.block,
                    color: theme.colorScheme.onErrorContainer,
                  ),
                  title: Text(
                    'Доступ отозван',
                    style: TextStyle(color: theme.colorScheme.onErrorContainer),
                  ),
                  subtitle: Text(
                    'Хост закрыл твой доступ. Запроси у администратора новую ссылку '
                    'и вставь её внизу экрана.',
                    style: TextStyle(color: theme.colorScheme.onErrorContainer),
                  ),
                ),
              ),
            ),
          Card(
            child: Column(
              children: [
                ListTile(
                  key: const Key('sub-host'),
                  leading: const Icon(Icons.dns_outlined),
                  title: const Text('Хост'),
                  subtitle: Text(sub.hostName ?? sub.hostId ?? '—'),
                ),
                const Divider(height: 1),
                ListTile(
                  key: const Key('sub-version'),
                  leading: const Icon(Icons.tag),
                  title: const Text('Версия подписки'),
                  subtitle: Text(sub.version == null ? '—' : '${sub.version}'),
                ),
                const Divider(height: 1),
                ListTile(
                  key: const Key('sub-updated'),
                  leading: const Icon(Icons.update),
                  title: const Text('Обновлена'),
                  subtitle: Text(updated),
                ),
                const Divider(height: 1),
                ListTile(
                  leading: const Icon(Icons.hub_outlined),
                  title: const Text('Движки'),
                  subtitle: Text(
                    engines.isEmpty ? '—' : engines,
                    key: const Key('sub-engines'),
                  ),
                ),
                const Divider(height: 1),
                ListTile(
                  leading: const Icon(Icons.rule),
                  title: Text(
                    'Наборы правил (включено $enabled из ${state.rules.sets.length})',
                  ),
                  subtitle: Text(
                    sets.isEmpty ? '—' : sets,
                    key: const Key('sub-sets'),
                  ),
                ),
              ],
            ),
          ),
          if (sub.lastRefreshError != null || stale.isNotEmpty) ...[
            const SizedBox(height: 12),
            Card(
              child: Column(
                children: [
                  if (sub.lastRefreshError != null)
                    ListTile(
                      key: const Key('sub-error'),
                      leading: Icon(
                        Icons.sync_problem,
                        color: theme.colorScheme.error,
                      ),
                      title: const Text(
                        'Последняя попытка обновления не удалась',
                      ),
                      subtitle: Text(sub.lastRefreshError!),
                    ),
                  if (stale.isNotEmpty)
                    ListTile(
                      key: const Key('sub-stale'),
                      leading: const Icon(Icons.history),
                      title: Text(
                        'Не обновлялась с ${formatWhen(stale.first.since.toLocal(), now: now)}',
                      ),
                      subtitle: const Text(
                        'Работает последняя сохранённая копия.',
                      ),
                    ),
                ],
              ),
            ),
          ],
          const SizedBox(height: 12),
          FilledButton.icon(
            key: const Key('refresh-now'),
            onPressed: controller.refreshing
                ? null
                : controller.refreshSubscription,
            icon: controller.refreshing
                ? const SizedBox.square(
                    dimension: 18,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  )
                : const Icon(Icons.refresh),
            label: const Text('Обновить сейчас'),
          ),
          const SectionHeader(
            'Ссылка подписки',
            subtitle:
                'Это твой доступ: не показывай и не отправляй её посторонним.',
          ),
          Wrap(
            spacing: 12,
            runSpacing: 8,
            children: [
              OutlinedButton.icon(
                key: const Key('link-show-qr'),
                onPressed: () => _showQr(context, controller),
                icon: const Icon(Icons.qr_code_2),
                label: const Text('Показать QR'),
              ),
              OutlinedButton.icon(
                key: const Key('link-copy'),
                onPressed: controller.copySubscriptionLink,
                icon: const Icon(Icons.copy),
                label: const Text('Скопировать ссылку'),
              ),
            ],
          ),
          const SectionHeader(
            'Сменить подписку',
            subtitle:
                'Вставь другую ссылку — после подтверждения подписка заменится целиком. '
                'Твои домены и подсети останутся.',
          ),
          TextField(
            key: const Key('new-link-field'),
            controller: _field,
            minLines: 1,
            maxLines: 3,
            keyboardType: TextInputType.url,
            textInputAction: TextInputAction.done,
            autocorrect: false,
            enableSuggestions: false,
            decoration: InputDecoration(
              labelText: 'Новая ссылка $appName',
              hintText: 'braid://…',
              errorText: _error,
              errorMaxLines: 3,
              suffixIcon: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  IconButton(
                    key: const Key('new-link-paste'),
                    tooltip: 'Вставить из буфера',
                    icon: const Icon(Icons.content_paste),
                    onPressed: _busy ? null : _paste,
                  ),
                  if (controller.caps.qrScan)
                    IconButton(
                      key: const Key('new-link-scan'),
                      tooltip: 'Сканировать QR',
                      icon: const Icon(Icons.qr_code_scanner),
                      onPressed: _busy ? null : _scan,
                    ),
                ],
              ),
            ),
            onChanged: (_) {
              if (_error != null) setState(() => _error = null);
            },
            onSubmitted: (_) => _apply(),
          ),
          const SizedBox(height: 8),
          Align(
            alignment: Alignment.centerRight,
            child: FilledButton.tonal(
              key: const Key('new-link-button'),
              onPressed: _busy ? null : _apply,
              child: const Text('Применить ссылку'),
            ),
          ),
        ],
      ),
    );
  }

  Future<void> _showQr(BuildContext context, BraidController controller) async {
    final link = await controller.subscriptionLink();
    if (!context.mounted) return;
    if (link == null) {
      controller.showNotice('Ссылка подписки недоступна');
      return;
    }
    await showDialog<void>(
      context: context,
      // Dialog, а не AlertDialog: QR строится через LayoutBuilder, а
      // AlertDialog меряет содержимое «насквозь».
      builder: (context) => Dialog(
        key: const Key('link-qr-dialog'),
        child: ConstrainedBox(
          constraints: const BoxConstraints(maxWidth: 360),
          child: Padding(
            padding: const EdgeInsets.all(24),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                Text(
                  'QR ссылки подписки',
                  style: Theme.of(context).textTheme.titleLarge,
                ),
                const SizedBox(height: 16),
                LinkQr(data: link),
                const SizedBox(height: 12),
                Text(
                  'Отсканируй его в $appName на другом устройстве. '
                  'Это твой доступ — не показывай посторонним.',
                  textAlign: TextAlign.center,
                  style: Theme.of(context).textTheme.bodySmall,
                ),
                const SizedBox(height: 16),
                Wrap(
                  alignment: WrapAlignment.end,
                  spacing: 8,
                  children: [
                    TextButton(
                      key: const Key('link-copy-dialog'),
                      onPressed: () {
                        Navigator.pop(context);
                        controller.copySubscriptionLink();
                      },
                      child: const Text('Копировать ссылку'),
                    ),
                    TextButton(
                      onPressed: () => Navigator.pop(context),
                      child: const Text('Закрыть'),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
