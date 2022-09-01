use core::ptr;
use core::time::Duration;

use esp_idf_sys::*;

use crate::delay::TickType;
use crate::interrupt;

#[inline(always)]
#[link_section = ".iram1.interrupt_task_do_yield"]
pub fn do_yield() {
    if interrupt::active() {
        #[cfg(esp32c3)]
        unsafe {
            if let Some(yielder) = interrupt::get_isr_yielder() {
                yielder();
            } else {
                vPortYieldFromISR();
            }
        }

        #[cfg(not(esp32c3))]
        unsafe {
            if let Some(yielder) = interrupt::get_isr_yielder() {
                yielder();
            } else {
                #[cfg(esp_idf_version_major = "4")]
                vPortEvaluateYieldFromISR(0);

                #[cfg(esp_idf_version_major = "5")]
                _frxt_setup_switch();
            }
        }
    } else {
        unsafe {
            vPortYield();
        }
    }
}

#[inline(always)]
#[link_section = ".iram1.interrupt_task_current"]
pub fn current() -> Option<TaskHandle_t> {
    if interrupt::active() {
        None
    } else {
        Some(unsafe { xTaskGetCurrentTaskHandle() })
    }
}

pub fn wait_any_notification() {
    loop {
        if let Some(notification) = wait_notification(None) {
            if notification != 0 {
                break;
            }
        }
    }
}

pub fn wait_notification(duration: Option<Duration>) -> Option<u32> {
    let mut notification = 0_u32;

    #[cfg(esp_idf_version = "4.3")]
    let notified = unsafe {
        xTaskNotifyWait(
            0,
            u32::MAX,
            &mut notification as *mut _,
            TickType::from(duration).0,
        )
    } != 0;

    #[cfg(not(esp_idf_version = "4.3"))]
    let notified = unsafe {
        xTaskGenericNotifyWait(
            0,
            0,
            u32::MAX,
            &mut notification as *mut _,
            TickType::from(duration).0,
        )
    } != 0;

    if notified {
        Some(notification)
    } else {
        None
    }
}

/// # Safety
///
/// When calling this function care should be taken to pass a valid
/// FreeRTOS task handle. Moreover, the FreeRTOS task should be valid
/// when this function is being called.
pub unsafe fn notify(task: TaskHandle_t, notification: u32) -> bool {
    let notified = if interrupt::active() {
        let mut higher_prio_task_woken: BaseType_t = Default::default();

        #[cfg(esp_idf_version = "4.3")]
        let notified = xTaskGenericNotifyFromISR(
            task,
            notification,
            eNotifyAction_eSetBits,
            ptr::null_mut(),
            &mut higher_prio_task_woken as *mut _,
        );

        #[cfg(not(esp_idf_version = "4.3"))]
        let notified = xTaskGenericNotifyFromISR(
            task,
            0,
            notification,
            eNotifyAction_eSetBits,
            ptr::null_mut(),
            &mut higher_prio_task_woken as *mut _,
        );

        if higher_prio_task_woken != 0 {
            do_yield();
        }

        notified
    } else {
        #[cfg(esp_idf_version = "4.3")]
        let notified =
            xTaskGenericNotify(task, notification, eNotifyAction_eSetBits, ptr::null_mut());

        #[cfg(not(esp_idf_version = "4.3"))]
        let notified = xTaskGenericNotify(
            task,
            0,
            notification,
            eNotifyAction_eSetBits,
            ptr::null_mut(),
        );

        notified
    };

    notified != 0
}

#[cfg(esp_idf_comp_pthread_enabled)]
pub mod thread_spawn {
    use esp_idf_sys::*;

    use crate::cpu::Core;

    #[derive(Debug)]
    pub struct ThreadSpawnConfiguration {
        pub name: &'static [u8],
        pub stack_size: usize,
        pub priority: u8,
        pub inherit: bool,
        pub pin_to_core: Option<Core>,
    }

    impl Default for ThreadSpawnConfiguration {
        fn default() -> Self {
            Self {
                name: b"thread\0",
                stack_size: PTHREAD_STACK_MIN as usize * 4,
                priority: 0,
                inherit: false,
                pin_to_core: None,
            }
        }
    }

    impl From<&ThreadSpawnConfiguration> for esp_pthread_cfg_t {
        fn from(conf: &ThreadSpawnConfiguration) -> Self {
            Self {
                thread_name: conf.name.as_ptr() as _,
                stack_size: conf.stack_size as _,
                prio: conf.priority as _,
                inherit_cfg: conf.inherit,
                pin_to_core: conf
                    .pin_to_core
                    .map(Into::into)
                    .unwrap_or(tskNO_AFFINITY as _),
            }
        }
    }

    impl From<esp_pthread_cfg_t> for ThreadSpawnConfiguration {
        fn from(conf: esp_pthread_cfg_t) -> Self {
            Self {
                name: unsafe {
                    core::slice::from_raw_parts(
                        conf.thread_name as _,
                        strlen(conf.thread_name) as usize + 1,
                    )
                },
                stack_size: conf.stack_size as _,
                priority: conf.prio as _,
                inherit: conf.inherit_cfg,
                pin_to_core: if conf.pin_to_core == tskNO_AFFINITY as _ {
                    None
                } else {
                    Some(conf.pin_to_core.into())
                },
            }
        }
    }

    pub fn get_default_conf() -> ThreadSpawnConfiguration {
        unsafe { esp_pthread_get_default_config() }.into()
    }

    pub fn get_conf() -> Option<ThreadSpawnConfiguration> {
        let mut conf: esp_pthread_cfg_t = Default::default();

        let res = unsafe { esp_pthread_get_cfg(&mut conf as _) };

        if res == ESP_ERR_NOT_FOUND {
            None
        } else {
            Some(conf.into())
        }
    }

    pub fn set_conf(conf: &ThreadSpawnConfiguration) -> Result<(), EspError> {
        esp!(unsafe { esp_pthread_set_cfg(&conf.into()) })?;

        Ok(())
    }
}
