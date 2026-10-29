import Cocoa
import FlutterMacOS

@main
class AppDelegate: FlutterAppDelegate {
  override func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
    return true
  }

  override func applicationSupportsSecureRestorableState(_ app: NSApplication) -> Bool {
    return true
  }

  /// Ссылки `braid://…` от системы (схема объявлена в `Info.plist`): и та, которой
  /// запустили приложение, и открытые при работающем приходят сюда. Дальше — каналу
  /// `org.braid/links`. `super` не зовём: движок Flutter раздаёт URL плагинам и своему
  /// deeplinking, а ссылка подписки не должна стать маршрутом UI (на Android то же —
  /// `flutter_deeplinking_enabled=false` в манифесте).
  override func application(_ application: NSApplication, open urls: [URL]) {
    for url in urls where url.scheme?.lowercased() == DeepLinkChannel.scheme {
      DeepLinkChannel.shared.deliver(url.absoluteString)
    }
  }
}

/// Канал `org.braid/links` — контракт §10 `interfaces.md`, Dart-сторона `ChannelDeepLinks`:
/// `initialLink() -> String?` — ссылка, которой запустили приложение; при холодном старте
/// система отдаёт её раньше, чем Dart готов её принять, поэтому до первого запроса она
/// хранится здесь (из нескольких остаётся последняя — это и есть намерение пользователя).
/// После ответа на `initialLink` каждая новая ссылка уходит событием `onLink(String)`.
/// Всё в главном потоке: и `application(_:open:)`, и обработчик канала.
final class DeepLinkChannel {
  static let name = "org.braid/links"
  static let scheme = "braid"
  static let shared = DeepLinkChannel()

  private var channel: FlutterMethodChannel?
  private var pending: String?
  /// Dart уже спрашивал `initialLink` — значит, слушает `onLink`.
  private var dartReady = false

  /// Привязать к движку главного окна (`MainFlutterWindow.awakeFromNib`).
  func attach(to controller: FlutterViewController) {
    let channel = FlutterMethodChannel(
      name: Self.name, binaryMessenger: controller.engine.binaryMessenger)
    channel.setMethodCallHandler { call, result in
      guard call.method == "initialLink" else {
        result(FlutterMethodNotImplemented)
        return
      }
      self.dartReady = true
      let link = self.pending
      self.pending = nil
      result(link)
    }
    self.channel = channel
  }

  /// Ссылка от системы: Dart готов — событие `onLink`, нет — ждёт `initialLink`.
  func deliver(_ link: String) {
    if dartReady, let channel {
      channel.invokeMethod("onLink", arguments: link)
    } else {
      pending = link
    }
  }
}
