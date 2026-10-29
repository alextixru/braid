/// Диалог, в котором идёт сценарий в браузере: вход через VK или капча.
///
/// Со встроенным браузером (Android) страница лежит в самом диалоге на весь
/// экран. С окном ОС (десктоп) страница открыта отдельно, а диалог говорит,
/// что идёт, и даёт отказаться: без него пользователь не знал бы, чего ждёт
/// приложение.
library;

import 'package:braid/services/captcha/web_flow.dart';
import 'package:braid/services/captcha/web_session.dart';
import 'package:flutter/material.dart';

/// Показать диалог поверх любого экрана и дождаться ответа сценария. Диалог
/// закрывается сам, когда сценарий завершился: ответом, закрытием окна,
/// временем или отказом пользователя.
Future<T> presentWebFlow<T>({
  required GlobalKey<NavigatorState> navigatorKey,
  required WebFlow<T> flow,
  required String title,
  required String hint,
  required String pageHint,
  Key? dialogKey,
}) async {
  final session = flow.session;
  // Сценарий не начался: показывать нечего, ответ уже готов.
  if (session == null) return flow.result;

  var context = navigatorKey.currentContext;
  if (context == null) {
    // Навигатор ещё не построен — ждём кадр.
    await WidgetsBinding.instance.endOfFrame;
    context = navigatorKey.currentContext;
  }
  if (context == null || !context.mounted) {
    // Некуда показать диалог — страницу не оставляем висеть.
    flow.cancel();
    return flow.result;
  }

  BuildContext? shownIn;
  final dialog = showDialog<void>(
    context: context,
    barrierDismissible: false,
    builder: (dialogContext) {
      shownIn = dialogContext;
      return WebFlowDialog(
        key: dialogKey,
        title: title,
        hint: hint,
        pageHint: pageHint,
        session: session,
        onCancel: flow.cancel,
      );
    },
  );
  final result = await flow.result;
  if (shownIn == null) await WidgetsBinding.instance.endOfFrame;
  final inner = shownIn;
  if (inner != null && inner.mounted) {
    final route = ModalRoute.of(inner);
    if (route != null && route.isActive) {
      Navigator.of(inner).removeRoute(route);
    }
  }
  // Диалог закрыт; его завершение ничего не несёт.
  dialog.ignore();
  return result;
}

class WebFlowDialog extends StatelessWidget {
  const WebFlowDialog({
    super.key,
    required this.title,
    required this.hint,
    required this.pageHint,
    required this.session,
    required this.onCancel,
  });

  final String title;

  /// Подсказка в диалоге, когда страница открыта отдельным окном ОС.
  final String hint;

  /// Подсказка над страницей, когда она встроена в сам диалог.
  final String pageHint;
  final WebSession session;

  /// Пользователь отказался: закрыл диалог, нажал «Отмена» или «Назад».
  final VoidCallback onCancel;

  @override
  Widget build(BuildContext context) {
    final page = session.embedded;
    return PopScope(
      // «Назад» на Android — тот же отказ, а не тихое закрытие диалога, под
      // которым сценарий продолжал бы ждать.
      canPop: false,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop) onCancel();
      },
      child: page == null ? _windowDialog(context) : _pageDialog(context, page),
    );
  }

  Widget _windowDialog(BuildContext context) {
    return AlertDialog(
      title: Text(title),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(hint),
          const SizedBox(height: 16),
          const LinearProgressIndicator(),
        ],
      ),
      actions: [
        TextButton(
          key: const Key('web-flow-cancel'),
          onPressed: onCancel,
          child: const Text('Отмена'),
        ),
      ],
    );
  }

  Widget _pageDialog(BuildContext context, Widget page) {
    return Dialog.fullscreen(
      child: Scaffold(
        appBar: AppBar(
          title: Text(title),
          leading: IconButton(
            key: const Key('web-flow-cancel'),
            tooltip: 'Отмена',
            icon: const Icon(Icons.close),
            onPressed: onCancel,
          ),
        ),
        body: Column(
          children: [
            Padding(
              padding: const EdgeInsets.fromLTRB(16, 8, 16, 8),
              child: Text(
                pageHint,
                style: Theme.of(context).textTheme.bodySmall,
              ),
            ),
            Expanded(child: page),
          ],
        ),
      ),
    );
  }
}
