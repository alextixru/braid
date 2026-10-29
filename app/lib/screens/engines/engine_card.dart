import 'package:braid/models/core_state.dart';
import 'package:braid/screens/engines/engine_texts.dart';
import 'package:braid/util/format.dart';
import 'package:flutter/material.dart';

/// Один движок: название, живость, отклик/потери/хендшейк, пометки «активный»,
/// «тёплый», «закреплён» и кнопка закрепления (ист. 22, 30, 31).
class EngineCard extends StatelessWidget {
  const EngineCard({
    super.key,
    required this.engine,
    required this.all,
    required this.active,
    required this.onTogglePin,
  });

  final EngineView engine;

  /// Все движки подписки: по ним различаются два движка одного вида.
  final List<EngineView> all;

  /// Движок несёт трафик.
  final bool active;

  /// Закрепить, а закреплённый — открепить (вернуть «Авто»).
  final VoidCallback onTogglePin;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    final scheme = theme.colorScheme;
    final health = engine.state == EngineStatus.stopped
        ? null
        : healthText(
            rttMs: engine.health.rttMs,
            lossPct: engine.health.lossPct,
            handshakeAgeS: engine.health.handshakeAgeS,
          );
    final tone = _tone(scheme);
    return Card(
      key: Key('engine-${engine.id}'),
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 12, 8, 8),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Row(
              children: [
                Icon(Icons.circle, size: 12, color: tone),
                const SizedBox(width: 8),
                Text(
                  engineLabel(engine, all),
                  style: theme.textTheme.titleMedium,
                ),
                const SizedBox(width: 8),
                Expanded(
                  child: Wrap(
                    spacing: 4,
                    runSpacing: 4,
                    children: [
                      if (active) const _Mark('активный'),
                      if (engine.warm) const _Mark('тёплый'),
                      if (engine.pinned) const _Mark('закреплён'),
                    ],
                  ),
                ),
              ],
            ),
            const SizedBox(height: 4),
            Text(
              engineStatusText(engine, active: active),
              style: theme.textTheme.bodyMedium?.copyWith(color: tone),
            ),
            if (health != null)
              Text(
                health,
                style: theme.textTheme.bodySmall?.copyWith(
                  color: scheme.onSurfaceVariant,
                ),
              ),
            Align(
              alignment: Alignment.centerRight,
              child: TextButton(
                key: Key('engine-pin-${engine.id}'),
                onPressed: onTogglePin,
                child: Text(engine.pinned ? 'Открепить' : 'Закрепить'),
              ),
            ),
          ],
        ),
      ),
    );
  }

  Color _tone(ColorScheme scheme) => switch (engine.state) {
    EngineStatus.connected => scheme.primary,
    EngineStatus.degraded || EngineStatus.waitingUser => scheme.tertiary,
    EngineStatus.dead => scheme.error,
    EngineStatus.starting || EngineStatus.stopped => scheme.outline,
  };
}

class _Mark extends StatelessWidget {
  const _Mark(this.label);

  final String label;

  @override
  Widget build(BuildContext context) {
    final theme = Theme.of(context);
    return DecoratedBox(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(8),
        color: theme.colorScheme.secondaryContainer,
      ),
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 2),
        child: Text(
          label,
          style: theme.textTheme.labelSmall?.copyWith(
            color: theme.colorScheme.onSecondaryContainer,
          ),
        ),
      ),
    );
  }
}
