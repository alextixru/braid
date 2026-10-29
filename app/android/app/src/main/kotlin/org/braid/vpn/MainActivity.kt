package org.braid.vpn

import android.app.Activity
import android.content.Intent
import android.net.VpnService
import android.os.Bundle
import android.util.Log
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel
import kotlin.concurrent.thread

/**
 * Канал `org.braid/vpn` для Flutter (контракт 09 ↔ 10): `prepare() -> bool` (системный
 * диалог согласия VPN), `startService() -> bool`, `stopService()`, `isServiceRunning() -> bool`,
 * `shareDomainPending() -> String?` (домен из share-меню). Сверх контракта —
 * `linkPending() -> String?`: ссылка `braid://`, которую ядро не приняло без подтверждения
 * замены хоста (`ReplaceRequested`); по оговорке тикета 10 — `initCore() -> bool` (ядро без
 * подключения, [initCore]) и `installedApps() -> List<Map>?` (приложения для правил
 * «по приложениям», [installedApps]). Всё остальное — через `braid_ffi` и `Core::global()`.
 *
 * Share-меню (`ACTION_SEND` text/plain) → домен ждёт подтверждения в UI;
 * deeplink `braid://` (`ACTION_VIEW`) → сервис → `Core::import_link`.
 */
class MainActivity : FlutterActivity() {
    companion object {
        const val CHANNEL = "org.braid/vpn"
        private const val TAG = "MainActivity"
        private const val REQUEST_VPN = 0x5650
    }

    private var pendingPrepare: MethodChannel.Result? = null

    @Volatile
    private var pendingDomain: String? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, CHANNEL).setMethodCallHandler { call, result ->
            when (call.method) {
                "prepare" -> prepare(result)
                "initCore" -> result.success(initCore())
                "installedApps" -> installedApps(result)
                "startService" -> {
                    BraidVpnService.start(this, BraidVpnService.ACTION_START)
                    result.success(true)
                }
                "stopService" -> {
                    if (BraidVpnService.isRunning) {
                        startService(BraidVpnService.intent(this, BraidVpnService.ACTION_STOP))
                    }
                    result.success(null)
                }
                "isServiceRunning" -> result.success(BraidVpnService.isRunning)
                "shareDomainPending" -> {
                    result.success(pendingDomain)
                    pendingDomain = null
                }
                "linkPending" -> result.success(BraidVpnService.takePendingLink())
                else -> result.notImplemented()
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        handleIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handleIntent(intent)
    }

    private fun handleIntent(intent: Intent?) {
        when (intent?.action) {
            Intent.ACTION_SEND -> if (intent.type == "text/plain") {
                DomainExtractor.extract(intent.getStringExtra(Intent.EXTRA_TEXT))?.let { pendingDomain = it }
            }
            Intent.ACTION_VIEW -> {
                val link = intent.data?.takeIf { it.scheme == "braid" }?.toString() ?: return
                BraidVpnService.start(this, BraidVpnService.ACTION_IMPORT_LINK, link)
            }
        }
    }

    /**
     * Поднять ядро без подключения: сервис стартует действием [BraidVpnService.ACTION_INIT]
     * (`nativeInit`, но не `connect`) — иначе окно при холодном старте без подписки получало
     * бы «Не удалось подключиться». Ядро уже собрано — успех без действий. `false` — сервис
     * не стартовал (приложение ушло в фон); окно попробует снова.
     */
    private fun initCore(): Boolean {
        if (BraidVpnService.coreReady) return true
        return try {
            BraidVpnService.start(this, BraidVpnService.ACTION_INIT)
            true
        } catch (e: IllegalStateException) {
            Log.w(TAG, "initCore: сервис не стартовал: ${e.message}")
            false
        }
    }

    /**
     * Установленные приложения для правил «по приложениям» — в фоне (`loadLabel` на сотнях
     * пакетов), форма ответа — [InstalledApps]. `null` — список не прочитался: окно предложит
     * ввести имя пакета вручную.
     */
    private fun installedApps(result: MethodChannel.Result) {
        val pm = packageManager
        val self = packageName
        thread(name = "braid-apps") {
            val apps = try {
                InstalledApps.forDart(InstalledApps.load(pm), self)
            } catch (e: Exception) {
                Log.w(TAG, "список приложений не прочитан: ${e.message}")
                null
            }
            runOnUiThread { result.success(apps) }
        }
    }

    /** Согласие на VPN — только из Activity (`VpnService.prepare`); один раз на установку. */
    private fun prepare(result: MethodChannel.Result) {
        val consent = VpnService.prepare(this)
        if (consent == null) {
            result.success(true)
            return
        }
        pendingPrepare?.success(false)
        pendingPrepare = result
        @Suppress("DEPRECATION")
        startActivityForResult(consent, REQUEST_VPN)
    }

    @Deprecated("Deprecated in Java")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        if (requestCode == REQUEST_VPN) {
            pendingPrepare?.success(resultCode == Activity.RESULT_OK)
            pendingPrepare = null
            return
        }
        @Suppress("DEPRECATION")
        super.onActivityResult(requestCode, resultCode, data)
    }
}
