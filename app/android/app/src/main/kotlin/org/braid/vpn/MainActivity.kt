package org.braid.vpn

import android.app.Activity
import android.content.Intent
import android.net.VpnService
import android.os.Bundle
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

/**
 * Канал `org.braid/vpn` для Flutter (контракт 09 ↔ 10): `prepare() -> bool` (системный
 * диалог согласия VPN), `startService() -> bool`, `stopService()`, `isServiceRunning() -> bool`,
 * `shareDomainPending() -> String?` (домен из share-меню). Сверх контракта —
 * `linkPending() -> String?`: ссылка `braid://`, которую ядро не приняло без подтверждения
 * замены хоста (`ReplaceRequested`). Всё остальное — через `braid_ffi` и `Core::global()`.
 *
 * Share-меню (`ACTION_SEND` text/plain) → домен ждёт подтверждения в UI;
 * deeplink `braid://` (`ACTION_VIEW`) → сервис → `Core::import_link`.
 */
class MainActivity : FlutterActivity() {
    companion object {
        const val CHANNEL = "org.braid/vpn"
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
