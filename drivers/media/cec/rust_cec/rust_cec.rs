// SPDX-License-Identifier: GPL-2.0

//! Rust CEC driver — Phase 5 full
//!
//! CVE-2023-52846 패턴을 Rust로 재구현하여 컴파일 타임 + 구조적 UAF 차단을 실증한다.
//!
//! C의 구조적 문제 → Rust 해결책:
//!   bool completed         → AtomicBool      (다중 스레드 공유 시 Sync 위반으로 컴파일 거부)
//!   kfree(data)            → Arc<CecData>    (workqueue가 Arc 보유 중이면 해제 불가)
//!   조건부 cancel_work     → Drop 트레이트   (소유자 소멸 시 무조건 실행, if 조건 불가)
//!   wait_for_completion    → Mutex + CondVar (안전한 블로킹 대기)

use core::sync::atomic::{AtomicBool, Ordering};
use kernel::prelude::*;
use kernel::sync::{new_condvar, new_mutex, Arc, CondVar, Mutex};
use kernel::workqueue::{self, impl_has_work, new_work, Work, WorkItem};

module! {
    type: RustCec,
    name: "rust_cec",
    author: "Leeharim-korean",
    description: "Rust CEC driver — compile-time UAF prevention (Phase 5)",
    license: "GPL",
}

/// C의 `struct cec_data`에 대응.
///
/// `#[pin_data]`: CondVar, Mutex, Work는 핀 고정 초기화(in-place init)가 필요하다.
/// Drop은 Arc 참조 카운트에 의해 암묵적으로 호출된다 (마지막 Arc 소멸 시).
/// 외부 모듈에서 PinnedDrop은 $crate 경로 문제로 사용하지 않는다.
#[pin_data]
struct CecData {
    /// C: `bool completed` (비원자적) → Rust: AtomicBool
    /// 일반 bool을 Arc로 감싸 스레드 간 공유하면 Sync 트레이트 위반으로 컴파일 거부된다.
    completed: AtomicBool,

    /// workqueue 완료 여부 (CondVar와 쌍으로 사용)
    #[pin]
    state: Mutex<bool>,

    /// C의 `struct completion`에 대응 — complete() / wait_for_completion_killable() 역할
    #[pin]
    done: CondVar,

    /// C의 `struct delayed_work`에 대응 — cec_wait_timeout() 시뮬레이션
    #[pin]
    work: Work<CecData>,
}

impl_has_work! {
    impl HasWork<Self> for CecData { self.work }
}

impl CecData {
    fn new() -> Result<Arc<Self>> {
        // Arc::pin_init: #[pin] 필드를 포함하는 구조체를 힙에 고정 할당
        // pin_init!의 <- 문법은 in-place 초기화 (이동 없이 해당 주소에서 직접 구성)
        Arc::pin_init(
            pin_init!(CecData {
                completed: AtomicBool::new(false),
                state <- new_mutex!(false),
                done <- new_condvar!(),
                work <- new_work!("CecData::work"),
            }),
            GFP_KERNEL,
        )
    }
}

/// workqueue 콜백 — C의 `cec_wait_timeout()`에 대응.
///
/// `type Pointer = Arc<CecData>`: workqueue는 Arc<CecData>를 소유권으로 받아 실행한다.
/// 실행 중 Arc 참조가 살아있으므로 외부에서 CecData를 해제할 수 없다.
/// C의 `kfree(data)` race가 구조적으로 불가능한 이유가 바로 이것이다.
impl WorkItem for CecData {
    type Pointer = Arc<CecData>;

    fn run(this: Arc<Self>) {
        pr_info!("cec_wait_timeout: running (Arc<CecData> held)\n");

        // 1. completed 플래그 설정 — Release: 이 쓰기가 다른 코어에 가시적임을 보장
        this.completed.store(true, Ordering::Release);

        // 2. 전송 스레드를 깨움 (C의 complete()에 대응)
        let mut guard = this.state.lock();
        *guard = true;
        this.done.notify_one();
        drop(guard);

        pr_info!("cec_wait_timeout: done\n");
        // `this` (Arc<CecData>)가 여기서 소멸.
        // 외부 Arc가 모두 소멸한 경우 → 이 시점에 CecData::drop() 호출.
        // C의 kfree(data)와 달리 workqueue 실행 중 절대 해제되지 않는다.
    }
}


struct RustCec {
    _data: Arc<CecData>,
}

impl kernel::Module for RustCec {
    fn init(_module: &'static ThisModule) -> Result<Self> {
        pr_info!("init: allocating CecData\n");
        let data = CecData::new()?;

        // workqueue에 Arc<CecData> 클론을 전달 — workqueue가 참조를 보유
        // C에서 kfree(data) race가 발생했던 이유: workqueue 완료 전에 해제 가능
        // Rust에서는 workqueue의 Arc가 살아있는 한 CecData 해제 자체가 불가능
        pr_info!("init: enqueueing work (Arc clone → workqueue)\n");
        let _ = workqueue::system().enqueue(data.clone());

        // 전송 완료 대기 (C의 wait_for_completion_killable에 대응)
        // while 루프: CondVar의 스퓨리어스 웨이크업(spurious wakeup) 방지
        {
            let mut guard = data.state.lock();
            while !*guard {
                data.done.wait(&mut guard);
            }
        }

        pr_info!(
            "init: transmit done, completed={}\n",
            data.completed.load(Ordering::Acquire)
        );

        Ok(Self { _data: data })
    }
}

impl Drop for RustCec {
    fn drop(&mut self) {
        pr_info!("RustCec::drop — _data Arc released, CecData freed if refcount→0\n");
        // _data(Arc<CecData>)가 여기서 소멸.
        // workqueue::run()이 이미 Arc를 해제했다면 이 시점에 CecData 메모리 해제.
        // C의 kfree(data) race와 달리: workqueue가 Arc를 보유 중이면 해제 자체가 불가능.
    }
}
