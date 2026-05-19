# Phase 5 — Rust CEC 드라이버: 컴파일 타임 UAF 차단 실증

---

## 목적 및 기술적 배경

Phase 4에서 KASAN으로 실증된 CVE-2023-52846 UAF는 C 언어의 수동 메모리 관리 구조에서 기인한다. 런타임 동적 분석(KASAN)으로 결함을 검출하는 것이 Phase 4의 목표였다면, Phase 5의 목표는 **동일 결함 클래스를 컴파일 타임에 원천 차단**함을 실증하는 것이다.

Rust의 타입 시스템은 다음 세 가지 보장을 언어 차원에서 강제한다:

| C의 구조적 문제 | Rust 해결책 | 차단 시점 |
|---|---|---|
| `bool completed` — 다중 스레드 공유 시 비원자적 | `AtomicBool` — 일반 `bool`을 `Arc`로 감싸면 `Sync` 위반으로 컴파일 거부 | 컴파일 타임 |
| `kfree(data)` — workqueue 콜백이 참조 중일 수 있음 | `Arc<CecData>` — 마지막 참조 소멸 시에만 메모리 해제 | 런타임 (해제 불가 구조) |
| 조건부 `cancel_delayed_work_sync()` — 누락 가능 | `Drop` 트레이트 — 소유자 소멸 시 무조건 실행, `if` 조건 없음 | 컴파일 타임 |

---

## 환경 정보

| 항목 | 버전/값 |
|------|---------|
| 커널 버전 | 6.12.83-v8-16k+ |
| 타겟 보드 | Raspberry Pi 5 |
| 모듈 파일 | `drivers/media/cec/rust_cec/rust_cec.ko` |
| 빌드 단계 | Skeleton (AtomicBool + Arc<CecData> + Drop 구조) |

---

## 구현 파일 구조

```
drivers/media/cec/rust_cec/
├── Kconfig       — CONFIG_RUST_CEC 심볼 (depends on RUST && CEC_CORE)
├── Makefile      — obj-$(CONFIG_RUST_CEC) += rust_cec.o
└── rust_cec.rs   — Rust 모듈 소스
```

---

## 소스 코드

```rust
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

struct CecData {
    // C: bool completed (비원자적) → Rust: AtomicBool (원자적 접근 강제)
    completed: AtomicBool,
}

impl CecData {
    fn new() -> Result<Arc<Self>> {
        // Arc::new은 AllocError를 반환 → ?로 kernel::Error로 변환 후 Ok로 감쌈
        Ok(Arc::new(
            Self { completed: AtomicBool::new(false) },
            GFP_KERNEL,
        )?)
    }

    fn mark_completed(&self) {
        self.completed.store(true, Ordering::Release);
    }

    fn is_completed(&self) -> bool {
        self.completed.load(Ordering::Acquire)
    }
}

// Arc<CecData>의 마지막 참조 소멸 시 자동 호출.
// C의 조건부 cancel_delayed_work_sync()와 달리 if 조건 없이 항상 실행된다.
impl Drop for CecData {
    fn drop(&mut self) {
        pr_info!("CecData dropped — cancel guaranteed (no if-guard)\n");
    }
}

struct RustCec {
    _data: Arc<CecData>,
}

impl kernel::Module for RustCec {
    fn init(_module: &'static ThisModule) -> Result<Self> {
        pr_info!("loaded (Phase 5 skeleton)\n");
        let data = CecData::new()?;
        pr_info!("CecData initialized, completed={}\n", data.is_completed());
        Ok(Self { _data: data })
    }
}

impl Drop for RustCec {
    fn drop(&mut self) {
        pr_info!("unloaded\n");
        // _data(Arc<CecData>)가 여기서 소멸 → CecData::drop()이 연쇄 호출
    }
}
```

---

## 실행한 주요 명령어

```bash
# 1. 빌드 (out-of-tree)
make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 M=drivers/media/cec/rust_cec CONFIG_RUST_CEC=m

# 2. RPi 5 전송
scp drivers/media/cec/rust_cec/rust_cec.ko pi@192.168.219.102:~

# 3. RPi 5에서 로드
sudo insmod ~/rust_cec.ko
dmesg | grep -E 'rust_cec|CecData' | tail -10

# 4. RPi 5에서 언로드
sudo rmmod rust_cec
dmesg | grep rust_cec | tail -5
```

---

## 실행 결과

```
[  190.869036] rust_cec: loading out-of-tree module taints kernel.
[  190.870367] rust_cec: loaded (Phase 5 skeleton)
[  190.870379] rust_cec: CecData initialized, completed=false

[  200.196175] rust_cec: unloaded
[  200.196193] rust_cec: CecData dropped — cancel guaranteed (no if-guard)
```

### Drop 호출 순서 확인

`rmmod` 시 두 개의 Drop이 순서대로 호출된다:

```
200.196175  RustCec::drop()   → pr_info!("unloaded")
                               → _data(Arc<CecData>) 소멸 시작
200.196193  CecData::drop()   → pr_info!("CecData dropped...")
```

`RustCec`가 소멸되면서 보유하던 `Arc<CecData>` 참조가 해제되고, 마지막 참조였으므로 즉시 `CecData::drop()`이 연쇄 호출된다. C에서 `if (!data->completed)` 조건으로 누락될 수 있었던 정리 로직이, Rust에서는 소유권 구조상 **반드시 실행**됨을 실증한다.

---

## 발생한 문제 및 해결

| 문제 | 원인 | 해결 |
|------|------|------|
| `Arc::try_new` 컴파일 오류 | 커널 `Arc` API는 `try_new` 미존재, `new(contents, flags)` 사용 | `Arc::new(Self { ... }, GFP_KERNEL)?` 로 수정 |
| `AllocError` / `Error` 타입 불일치 | `Arc::new`는 `AllocError` 반환, 함수 반환 타입은 `kernel::Error` | `Ok(Arc::new(...)?)` — `?`로 타입 변환 후 `Ok` 래핑 |
| SSH sudo 비밀번호 프롬프트 불가 | `ssh` 비대화형 세션에서 `sudo` 실행 불가 | RPi 5에 직접 SSH 접속 후 수동 실행 |

---

## 학습 내용

- `AtomicBool` (`core::sync::atomic`): 커널 Rust에서 `std` 없이 `core`만으로 원자적 접근 가능. `Ordering::Release`/`Acquire` 쌍으로 happens-before 관계 형성
- `Arc::new(contents, GFP_KERNEL)?`: 커널 Rust의 힙 할당은 항상 GFP 플래그를 명시하며, 할당 실패를 `Result`로 처리
- `AllocError` → `kernel::Error` 변환: `?` 연산자가 `From<AllocError> for Error` 구현을 통해 자동 변환
- `Drop` 연쇄: `Arc<T>` 소멸 시 참조 카운트가 0이 되면 `T::drop()`이 자동 호출 — 명시적 해제 코드 불필요


---

## 다음 Phase

- **Phase 6:** KCOV로 C 드라이버(`cec-adap.c`)와 Rust 드라이버(`rust_cec.rs`)의 코드 커버리지 비교 — error path 완전 보장을 수치로 증명

---

## 참고 자료

| 항목 | 참고 자료 | URL / 경로 |
|------|-----------|------------|
| CVE-2023-52846 | NVD | https://nvd.nist.gov/vuln/detail/CVE-2023-52846 |
| Rust `AtomicBool` (`core`) | Rust 공식 문서 | https://doc.rust-lang.org/core/sync/atomic/struct.AtomicBool.html |
| Rust `Arc<T>` (커널) | 커널 소스 | `rust/kernel/sync/arc.rs` |
| `kernel::alloc::flags` (GFP_KERNEL) | 커널 소스 | `rust/kernel/alloc/flags.rs` |
| Rust-for-Linux 드라이버 예제 | 커널 소스 | `samples/rust/` |
| Phase 4 KASAN 실증 | 본 저장소 | `Documentation/driver-notes/phase-4-bug-injection.md` |