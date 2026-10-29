package org.braid.vpn

import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.net.VpnService
import android.os.Build
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.PowerManager
import android.system.OsConstants
import android.util.Log
import org.json.JSONObject
import java.io.File
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.RejectedExecutionException

/**
 * Foreground-сервис VPN (ист. 3, 9, 10, 42, 43, 55, 57). Ядро Rust живёт в процессе
 * приложения и поднимается отсюда (`nativeInit`) независимо от Flutter engine — так
 * работают always-on (`SERVICE_INTERFACE`) и старт после перезагрузки (`BootReceiver`).
 *
 * Сервис делает ровно то, чего ядро не может само: `establish()` по описанию из Rust
 * ([requestEstablish]), `protect(fd)` для сокетов движков (унаследован от [VpnService]),
 * уведомления ([notify], [updateNotification]), флаг автозапуска ([setAutostart]).
 * События системы — сеть, экран, `onRevoke` — уходят в ядро native-вызовами.
 *
 * Жизнь сервиса = жизнь сессии: он уходит по «Отключить», по `stopService` из
 * приложения и когда ядро сообщает, что сессии больше нет (`idle`, `needs_link`,
 * `interrupted`). Native-вызовы идут в одном рабочем потоке: `connect()` ядра ходит в
 * `establish()` и запускает движки — главному потоку там делать нечего.
 */
class BraidVpnService : VpnService() {
    companion object {
        private const val TAG = "BraidVpnService"

        /** Настройки сервиса (`BootReceiver` читает флаг автозапуска). */
        const val PREFS = "braid.service"
        const val PREF_AUTOSTART = "autostart"
        const val EXTRA_LINK = "link"

        const val ACTION_START = "org.braid.vpn.START"
        const val ACTION_STOP = "org.braid.vpn.STOP"
        const val ACTION_RESUME = "org.braid.vpn.RESUME"
        const val ACTION_DISCONNECT = "org.braid.vpn.DISCONNECT"
        const val ACTION_CYCLE_ENGINE = "org.braid.vpn.CYCLE_ENGINE"
        const val ACTION_IMPORT_LINK = "org.braid.vpn.IMPORT_LINK"

        /** Коды [requestEstablish] < 0 — те же, что `ESTABLISH_*` в `jni_service.rs`. */
        const val ESTABLISH_NULL = -1
        const val ESTABLISH_NOT_PREPARED = -2
        const val ESTABLISH_BUILDER = -3

        /** Каталог хранилища ядра — вне резервных копий: там ключи и токен подписки. */
        private const val STORAGE_DIR = "braid"

        @Volatile
        var isRunning: Boolean = false
            private set

        /** Ссылка `braid://`, которую ядро не приняло без подтверждения замены хоста. */
        @Volatile
        private var pendingLink: String? = null

        fun takePendingLink(): String? = pendingLink.also { pendingLink = null }

        init {
            // Библиотека моста: JNI_OnLoad в ней регистрирует мост ядра для native-методов.
            System.loadLibrary("braid_ffi")
        }

        fun intent(context: Context, action: String, link: String? = null): Intent =
            Intent(context, BraidVpnService::class.java).setAction(action).apply {
                link?.let { putExtra(EXTRA_LINK, it) }
            }

        /** Старт/возобновление — foreground; остальное (стоп, ссылка) — обычная команда. */
        fun start(context: Context, action: String, link: String? = null) {
            val intent = intent(context, action, link)
            when (action) {
                ACTION_START, ACTION_RESUME -> context.startForegroundService(intent)
                else -> context.startService(intent)
            }
        }
    }

    // ---------- Kotlin → Rust (exports.rs) ----------

    private external fun nativeInit(storageDir: String): Boolean

    /** Контракт `interfaces.md`: fd от сервиса в ядро. Сейчас fd нового интерфейса уходит
     * из [requestEstablish] синхронно, вход остаётся для установки по инициативе сервиса. */
    @Suppress("unused")
    private external fun nativeOnTunFd(fd: Int, mtu: Int)
    private external fun nativeNetworkChanged(id: String, kind: String)
    private external fun nativeNetworkLost()
    private external fun nativeOriginalDns(dns: Array<String>)
    private external fun nativeStop()
    private external fun nativeRevoked()
    private external fun nativeConnect(): String?
    private external fun nativeResume(): Boolean
    private external fun nativeDisconnect()
    private external fun nativeImportLink(link: String): String
    private external fun nativeCycleEngine()
    private external fun nativeSetIdle(idle: Boolean)

    private val main = Handler(Looper.getMainLooper())
    private val worker: ExecutorService = Executors.newSingleThreadExecutor { Thread(it, "braid-service") }
    private lateinit var notifications: VpnNotifications
    private lateinit var watcher: NetworkWatcher

    @Volatile
    private var initialized = false

    @Volatile
    private var inForeground = false

    /** Ядро уже показывало живую сессию — её конец означает, что сервису пора уходить. */
    @Volatile
    private var sessionSeen = false

    @Volatile
    private var lastView: ServiceView? = null

    private val screen = object : BroadcastReceiver() {
        override fun onReceive(context: Context, intent: Intent) {
            val idle = intent.action == Intent.ACTION_SCREEN_OFF
            native { if (initialized) nativeSetIdle(idle) }
        }
    }

    override fun onCreate() {
        super.onCreate()
        isRunning = true
        notifications = VpnNotifications(this)
        watcher = NetworkWatcher(
            this,
            onChanged = { id, kind -> native { nativeNetworkChanged(id, kind) } },
            onLost = { native { nativeNetworkLost() } },
            onDns = { dns -> native { nativeOriginalDns(dns.toTypedArray()) } },
        )
        val filter = IntentFilter(Intent.ACTION_SCREEN_OFF).apply { addAction(Intent.ACTION_SCREEN_ON) }
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(screen, filter, Context.RECEIVER_NOT_EXPORTED)
        } else {
            registerReceiver(screen, filter)
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        // intent == null — сервис перезапущен системой после смерти (START_STICKY):
        // ведём себя как при always-on.
        val action = intent?.action ?: SERVICE_INTERFACE
        Log.i(TAG, "команда $action")
        when (action) {
            SERVICE_INTERFACE, ACTION_START -> {
                foreground()
                native { connect() }
                return START_STICKY
            }
            ACTION_RESUME -> {
                foreground()
                native { if (!resume()) stop() }
                return START_STICKY
            }
            ACTION_STOP, ACTION_DISCONNECT -> {
                // Из уведомления (getForegroundService) сервис обязан выйти в foreground,
                // даже если тут же уходит.
                foreground()
                native {
                    if (initialized) nativeDisconnect()
                    stop()
                }
            }
            ACTION_CYCLE_ENGINE -> {
                foreground()
                native { if (ensureInit()) nativeCycleEngine() }
            }
            ACTION_IMPORT_LINK -> {
                // Сервис поднят только ради ссылки: уходит, если сессии нет и ядро её не
                // подняло само (восстановление после перезагрузки стартует при nativeInit).
                val link = intent?.getStringExtra(EXTRA_LINK)
                native {
                    importLink(link)
                    main.post { if (!inForeground && !sessionSeen) stopSelf(startId) }
                }
            }
            else -> Log.w(TAG, "неизвестная команда $action")
        }
        return START_NOT_STICKY
    }

    /** Always-on привязывается к сервису действием `SERVICE_INTERFACE`; остальным — нечего. */
    override fun onBind(intent: Intent?): IBinder? =
        if (intent?.action == SERVICE_INTERFACE) super.onBind(intent) else null

    /** Интерфейс забрал другой VPN или разрешение отозвано (ист. 9): ядро — в «перехвачен». */
    override fun onRevoke() {
        Log.w(TAG, "VPN перехвачен другим приложением или разрешение отозвано")
        native { if (initialized) nativeRevoked() }
        notifications.event("VPN перехвачен", "Другое приложение забрало VPN. Откройте Braid, чтобы вернуть.")
        stop()
    }

    override fun onDestroy() {
        isRunning = false
        inForeground = false
        try {
            unregisterReceiver(screen)
        } catch (_: IllegalArgumentException) {
            // не был зарегистрирован
        }
        watcher.stop()
        worker.execute {
            if (initialized) {
                try {
                    nativeStop()
                } catch (e: Throwable) {
                    Log.e(TAG, "nativeStop: ${e.message}")
                }
            }
        }
        worker.shutdown()
        super.onDestroy()
    }

    // ---------- Rust → Kotlin (jni_service.rs); зовутся из потоков ядра ----------

    /**
     * `VpnService.Builder` по описанию из Rust → `establish()` → `detachFd()`:
     * владение дескриптором переходит ядру. Возвращает fd ≥ 0 или код `ESTABLISH_*`.
     * При неудаче прежний интерфейс остаётся как был — так обещает `establish()`.
     */
    fun requestEstablish(json: String): Int {
        val spec = try {
            VpnInterfaceSpec.fromJson(json, packageName)
        } catch (e: Exception) {
            Log.w(TAG, "описание интерфейса отвергнуто: ${e.message}")
            return ESTABLISH_BUILDER
        }
        val builder = Builder()
            .setSession("Braid")
            .setConfigureIntent(openApp())
            .setMetered(false)
        try {
            spec.ops.forEach { applyOp(builder, it) }
        } catch (e: IllegalArgumentException) {
            Log.w(TAG, "Builder отверг описание: ${e.message}")
            return ESTABLISH_BUILDER
        }
        val tun = try {
            builder.establish()
        } catch (e: SecurityException) {
            Log.w(TAG, "нет согласия на VPN: нужен prepare() из приложения")
            return ESTABLISH_NOT_PREPARED
        } catch (e: IllegalStateException) {
            Log.w(TAG, "establish(): ${e.message}")
            return ESTABLISH_BUILDER
        } catch (e: IllegalArgumentException) {
            Log.w(TAG, "establish(): ${e.message}")
            return ESTABLISH_BUILDER
        }
        if (tun == null) {
            Log.w(TAG, "establish() вернул null: разрешение VPN не выдано или отозвано")
            return ESTABLISH_NULL
        }
        Log.i(TAG, "интерфейс установлен: маршрутов ${spec.routes.size}, mtu ${spec.mtu}")
        return tun.detachFd()
    }

    private fun applyOp(builder: Builder, op: BuilderOp) {
        when (op) {
            is BuilderOp.Address -> builder.addAddress(op.address, op.prefix)
            is BuilderOp.Mtu -> builder.setMtu(op.mtu)
            is BuilderOp.Dns -> builder.addDnsServer(op.server)
            is BuilderOp.Route -> builder.addRoute(op.address, op.prefix)
            BuilderOp.AllowIpv6 -> builder.allowFamily(OsConstants.AF_INET6)
            is BuilderOp.AllowApp -> try {
                builder.addAllowedApplication(op.packageName)
            } catch (_: PackageManager.NameNotFoundException) {
                Log.w(TAG, "приложение из режима «по приложениям» не установлено — пропущено")
            }
            is BuilderOp.DisallowApp -> try {
                builder.addDisallowedApplication(op.packageName)
            } catch (_: PackageManager.NameNotFoundException) {
                Log.w(TAG, "исключённое приложение не установлено — пропущено")
            }
        }
    }

    /** Разовое уведомление ОС (`Platform::notify`): `{"title":…,"body":…}`. */
    fun notify(json: String) {
        val o = try {
            JSONObject(json)
        } catch (e: Exception) {
            return
        }
        notifications.event(o.optString("title", "Braid"), o.optString("body"))
    }

    /**
     * Состояние ядра для постоянного уведомления (`NotificationView`). Появление сессии
     * выводит сервис в foreground (ядро могло поднять её само — после перезагрузки или
     * импорта ссылки), её конец после живой сессии — останавливает сервис.
     */
    fun updateNotification(json: String) {
        val view = try {
            ServiceView.fromJson(json)
        } catch (e: Exception) {
            return
        }
        lastView = view
        when (view.state) {
            "connecting", "connected" -> {
                sessionSeen = true
                if (!inForeground) foreground()
            }
            "idle", "needs_link", "interrupted" -> if (sessionSeen && inForeground) {
                stop()
                return
            }
        }
        if (inForeground) notifications.update(view)
    }

    /**
     * `Platform::set_autostart`: флаг для `BootReceiver` (ист. 55). Надёжнее системный
     * always-on — включить его может только пользователь в настройках Android, поэтому
     * при включении автозапуска без always-on сервис подсказывает.
     */
    fun setAutostart(on: Boolean) {
        getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit().putBoolean(PREF_AUTOSTART, on).apply()
        if (on && !isAlwaysOn) {
            notifications.event(
                "Автозапуск включён",
                "Чтобы VPN поднимался сразу после перезагрузки, включите «Постоянная VPN» для Braid в настройках Android.",
            )
        }
    }

    // ---------- внутреннее ----------

    /** Запустить в рабочем потоке; ошибка native-вызова не роняет сервис. */
    private fun native(block: () -> Unit) {
        try {
            worker.execute {
                try {
                    block()
                } catch (e: Throwable) {
                    Log.e(TAG, "native: ${e.message}")
                }
            }
        } catch (_: RejectedExecutionException) {
            // сервис уже уничтожен — поздний колбэк сети или экрана
        }
    }

    /** Поднять ядро один раз на экземпляр сервиса (повторный nativeInit привязывает новый экземпляр). */
    private fun ensureInit(): Boolean {
        if (initialized) return true
        val dir = File(noBackupFilesDir, STORAGE_DIR).apply { mkdirs() }
        val ok = try {
            nativeInit(dir.absolutePath)
        } catch (e: UnsatisfiedLinkError) {
            Log.e(TAG, "native-методы не найдены: ${e.message}")
            false
        }
        if (!ok) {
            notifications.event("Braid", "Ядро VPN не запустилось")
            return false
        }
        initialized = true
        watcher.start()
        val power = getSystemService(PowerManager::class.java)
        nativeSetIdle(power?.isInteractive == false)
        return true
    }

    private fun connect() {
        if (!ensureInit()) {
            stop()
            return
        }
        val error = nativeConnect() ?: return
        Log.w(TAG, "подключение не удалось: $error")
        notifications.event("Не удалось подключиться", error)
        stop()
    }

    /** После перезагрузки: ядро само решает, была ли сессия активна (`Core::resume`). */
    private fun resume(): Boolean = ensureInit() && nativeResume()

    /** Deeplink `braid://` → `Core::import_link`; замену хоста подтверждает приложение. */
    private fun importLink(link: String?) {
        if (link.isNullOrEmpty() || !ensureInit()) return
        val answer = try {
            JSONObject(nativeImportLink(link))
        } catch (e: Exception) {
            Log.w(TAG, "импорт ссылки: ${e.message}")
            return
        }
        when {
            answer.has("replace") -> pendingLink = link
            answer.has("error") -> notifications.event("Ссылка не принята", answer.optString("error"))
        }
    }

    @Synchronized
    private fun foreground() {
        if (inForeground) return
        val notification = notifications.service(lastView)
        try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                startForeground(
                    VpnNotifications.SERVICE_ID,
                    notification,
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_SYSTEM_EXEMPTED,
                )
            } else {
                startForeground(VpnNotifications.SERVICE_ID, notification)
            }
            inForeground = true
        } catch (e: Exception) {
            Log.e(TAG, "startForeground: ${e.message}")
        }
    }

    private fun stop() {
        main.post {
            inForeground = false
            stopForeground(STOP_FOREGROUND_REMOVE)
            stopSelf()
        }
    }

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        this,
        0,
        Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )
}
