package com.sisyphus

import android.os.Bundle
import androidx.activity.enableEdgeToEdge
import com.sisyphus.notification.NotificationChannels

class MainActivity : TauriActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        NotificationChannels.createAll(this)
    }
}
