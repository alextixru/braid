package org.braid.vpn

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.graphics.drawable.Icon

/**
 * Уведомления сервиса (ист. 57, G08): постоянное — движок и состояние с действиями
 * «Отключить» и «Движок: <следующий>»; событийные — из `Platform::notify` ядра.
 */
class VpnNotifications(private val context: Context) {
    companion object {
        const val CHANNEL_SERVICE = "braid.vpn"
        const val CHANNEL_EVENTS = "braid.events"
        const val SERVICE_ID = 1
        private const val EVENTS_FROM = 100
    }

    private val manager = context.getSystemService(NotificationManager::class.java)
    private var eventId = EVENTS_FROM

    init {
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_SERVICE, "Состояние VPN", NotificationManager.IMPORTANCE_LOW)
                .apply { setShowBadge(false) },
        )
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_EVENTS, "События", NotificationManager.IMPORTANCE_DEFAULT),
        )
    }

    /** Постоянное уведомление foreground-сервиса. */
    fun service(view: ServiceView?): Notification {
        val v = view ?: ServiceView.starting()
        val builder = Notification.Builder(context, CHANNEL_SERVICE)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(v.text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setCategory(Notification.CATEGORY_SERVICE)
            .setVisibility(Notification.VISIBILITY_PUBLIC)
            .setContentIntent(openApp())
        v.detail?.let { builder.setContentText(it) }
        builder.addAction(action("Отключить", BraidVpnService.ACTION_DISCONNECT, 1))
        if (v.canCycle) {
            builder.addAction(action("Движок: ${v.nextLabel}", BraidVpnService.ACTION_CYCLE_ENGINE, 2))
        }
        return builder.build()
    }

    fun update(view: ServiceView) {
        manager.notify(SERVICE_ID, service(view))
    }

    /** Разовое уведомление ОС (`Platform::notify`). */
    fun event(title: String, body: String) {
        val notification = Notification.Builder(context, CHANNEL_EVENTS)
            .setSmallIcon(R.mipmap.ic_launcher)
            .setContentTitle(title)
            .setContentText(body)
            .setAutoCancel(true)
            .setContentIntent(openApp())
            .build()
        manager.notify(eventId++, notification)
    }

    private fun openApp(): PendingIntent = PendingIntent.getActivity(
        context,
        0,
        Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
        PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
    )

    private fun action(label: String, action: String, code: Int): Notification.Action {
        val pending = PendingIntent.getForegroundService(
            context,
            code,
            BraidVpnService.intent(context, action),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return Notification.Action.Builder(
            Icon.createWithResource(context, R.mipmap.ic_launcher),
            label,
            pending,
        ).build()
    }
}
