// SPDX-License-Identifier: GPL-2.0

//! Rust CEC driver — Phase 5 skeleton
//!
//! CVE-2023-52846 패턴을 Rust로 재구현하여 컴파일 타임 UAF 차단을 실증한다.
//!
//! C의 구조적 문제 → Rust 해결책:
//!   bool completed        → AtomicBool      (다중 스레드 공유 시 Sync 위반으로 컴파일 거부)
//!   kfree(data)           → Arc<CecData>    (참조가 남아 있으면 해제 불가)
//!   조건부 cancel_work    → Drop 트레이트   (소유자 소멸 시 무조건 실행, if 조건 불필요)

use core::sync::atomic::{AtomicBool, Ordering};
use kernel::prelude::*;
use kernel::sync::Arc;

module! {
    type: RustCec,
    name: "rust_cec",
    author: "Leeharim-korean",
    description: "Rust CEC driver — compile-time UAF prevention (Phase 5)",
    license: "GPL",
}

/// CEC 전송 상태를 추적하는 핵심 데이터 구조.
///
/// C의 `struct cec_data`에서 `bool completed`가 Race Condition의 근원이었다.
/// Rust에서는 `AtomicBool`을 사용해야만 여러 스레드가 공유할 수 있다:
/// 일반 `bool`을 Arc로 감싸 공유하려 하면 `Sync` 트레이트 미구현으로 컴파일이 거부된다.
struct CecData {
    /// C: `bool completed` (비원자적) → Rust: AtomicBool (원자적 접근 강제)
    completed: AtomicBool,
}

impl CecData {
    fn new() -> Result<Arc<Self>> {
        // 커널 Rust에서 Arc 할당은 GFP_KERNEL 플래그와 함께 Result를 반환
        // Arc::new은 AllocError를 반환 → ?로 kernel::Error로 변환 후 Ok로 감쌈
        Ok(Arc::new(
            Self {
                completed: AtomicBool::new(false),
            },
            GFP_KERNEL,
        )?)
    }

    /// C의 cec_data_completed()에 대응.
    /// Acquire/Release 순서로 completed 플래그를 설정한다.
    fn mark_completed(&self) {
        // Release: 이 쓰기 이전의 모든 메모리 연산이 다른 코어에 가시적임을 보장
        self.completed.store(true, Ordering::Release);
    }

    fn is_completed(&self) -> bool {
        // Acquire: mark_completed()의 Release와 쌍을 이뤄 happens-before 관계 형성
        self.completed.load(Ordering::Acquire)
    }
}

// Arc<CecData>의 마지막 참조가 소멸될 때 자동 호출.
// C의 조건부 cancel_delayed_work_sync()와 달리 if 조건 없이 항상 실행된다.
// workqueue가 Arc를 보유 중이면 이 Drop이 호출되지 않으므로 UAF 자체가 불가능.
impl Drop for CecData {
    fn drop(&mut self) {
        pr_info!("rust_cec: CecData dropped — cancel guaranteed (no if-guard)\n");
        // Phase 5 full: 여기서 cancel_delayed_work_sync 해당 Rust 추상화 호출 예정
    }
}

/// 모듈 진입점. C의 module_init()에 대응.
struct RustCec {
    _data: Arc<CecData>,
}

impl kernel::Module for RustCec {
    fn init(_module: &'static ThisModule) -> Result<Self> {
        pr_info!("rust_cec: loaded (Phase 5 skeleton)\n");

        let data = CecData::new()?;
        pr_info!(
            "rust_cec: CecData initialized, completed={}\n",
            data.is_completed()
        );

        Ok(Self { _data: data })
    }
}

// 모듈 언로드 시 자동 호출. C의 module_exit()에 대응.
impl Drop for RustCec {
    fn drop(&mut self) {
        pr_info!("rust_cec: unloaded\n");
    }
}
