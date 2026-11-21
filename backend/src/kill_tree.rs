//! ฆ่า "ทั้งต้นไม้โปรเซส" ของคำสั่งที่ exec.run เริ่ม
//!
//! กับดัก: `child.kill()` ฆ่าแค่ shell (powershell/sh) — โปรเซสลูกอย่าง `ping -t` ที่ shell เปิดไว้ยังรันค้างต่อไป
//! Windows: ใส่ shell เข้า Job Object ที่ตั้ง KILL_ON_JOB_CLOSE แล้วปิด handle = ฆ่าทุกตัวใน job
//! (shortcut: ใส่เข้า job หลัง spawn ไม่ใช่ตอนสร้าง — ลูกที่ shell เปิดภายในไม่กี่มิลลิวินาทีแรกอาจหลุด job)
//! Unix: คำสั่งถูกสร้างเป็น process group ของตัวเอง แล้วส่ง SIGKILL ให้ทั้งกลุ่ม (ยังไม่ได้ทดสอบบน Unix)
//!
//! ใช้ `release()` เมื่อคำสั่งจบเองตามปกติ — จะไม่ฆ่าอะไร (คำสั่งที่ตั้งใจเปิดโปรแกรมทิ้งไว้เบื้องหลังต้องไม่ถูกฆ่าตอนจบ)

pub struct TreeGuard {
    #[cfg(windows)]
    job: Option<windows::Win32::Foundation::HANDLE>,
    #[cfg(unix)]
    pid: Option<u32>,
}

#[cfg(windows)]
mod imp {
    use super::TreeGuard;
    use std::{ffi::c_void, mem::size_of};
    use windows::{
        Win32::{
            Foundation::{CloseHandle, HANDLE},
            System::JobObjects::{
                AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
                JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation, SetInformationJobObject,
            },
        },
        core::PCWSTR,
    };

    // HANDLE เป็น pointer ดิบจึง !Send — job handle ใช้ข้าม thread ได้ตามปกติ
    unsafe impl Send for TreeGuard {}

    fn set_kill_on_close(job: HANDLE, on: bool) -> windows::core::Result<()> {
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        if on {
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        }
        unsafe {
            SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const c_void,
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
    }

    impl TreeGuard {
        pub fn attach(child: &tokio::process::Child) -> TreeGuard {
            let job = (|| -> windows::core::Result<HANDLE> {
                let process = child.raw_handle().ok_or_else(windows::core::Error::empty)?;
                let job = unsafe { CreateJobObjectW(None, PCWSTR::null())? };
                set_kill_on_close(job, true)?;
                unsafe { AssignProcessToJobObject(job, HANDLE(process))? };
                Ok(job)
            })();
            // ผูกไม่สำเร็จ (เช่น โปรเซสจบไปแล้ว): ยังมี kill_on_drop ของ tokio ฆ่า shell ได้ แต่ลูกอาจค้าง
            TreeGuard {
                job: job.inspect_err(|e| tracing::warn!("ผูก Job Object ไม่สำเร็จ: {e}")).ok(),
            }
        }

        pub fn release(mut self) {
            if let Some(job) = self.job.take() {
                let _ = set_kill_on_close(job, false);
                let _ = unsafe { CloseHandle(job) };
            }
        }
    }

    impl Drop for TreeGuard {
        fn drop(&mut self) {
            if let Some(job) = self.job.take() {
                let _ = unsafe { CloseHandle(job) }; // KILL_ON_JOB_CLOSE ฆ่าทุกตัวใน job
            }
        }
    }
}

#[cfg(unix)]
mod imp {
    use super::TreeGuard;

    impl TreeGuard {
        pub fn attach(child: &tokio::process::Child) -> TreeGuard {
            TreeGuard { pid: child.id() }
        }

        pub fn release(mut self) {
            self.pid = None;
        }
    }

    impl Drop for TreeGuard {
        fn drop(&mut self) {
            if let Some(pid) = self.pid.take() {
                // `kill -KILL -<pgid>` = ส่งให้ทั้งกลุ่ม (exec.rs สร้างคำสั่งเป็นกลุ่มของตัวเองด้วย process_group(0))
                let _ = std::process::Command::new("kill")
                    .args(["-KILL", &format!("-{pid}")])
                    .status();
            }
        }
    }
}
