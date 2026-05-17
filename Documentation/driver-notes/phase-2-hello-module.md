# Phase 2 — Hello Rust Kernel Module

---

## 환경 정보

| 항목 | 버전/값 |
|------|---------|
| 커널 버전 | 6.12.83-v8-16k+ |
| 타겟 보드 | Raspberry Pi 5 |
| 모듈 파일 | `drivers/hello_rust/hello_rust.ko` |

---

## 구현 파일 구조

```
drivers/hello_rust/
├── Kconfig        — CONFIG_HELLO_RUST 심볼 정의 (depends on RUST)
├── Makefile       — obj-$(CONFIG_HELLO_RUST) += hello_rust.o
└── hello_rust.rs  — Rust 모듈 소스
```

## 소스 코드 요약

```rust
use kernel::prelude::*;

module! {
    type: HelloRust,
    name: "hello_rust",
    author: "Leeharim-korean",
    description: "Hello Rust kernel module for RPi 5 (Phase 2)",
    license: "GPL",
}

struct HelloRust;

impl kernel::Module for HelloRust {
    fn init(_module: &'static ThisModule) -> Result<Self> {
        pr_info!("Hello from Rust! (init)\n");
        pr_info!("Rust module loaded on Raspberry Pi 5\n");
        Ok(HelloRust)
    }
}

impl Drop for HelloRust {
    fn drop(&mut self) {
        pr_info!("Goodbye from Rust! (exit)\n");
    }
}
```

---

## 실행한 주요 명령어

```bash
# 1. 빌드 (out-of-tree 모듈)
# out-of-tree: 커널 전체를 다시 빌드하지 않고, M= 으로 지정한 디렉토리만
# 독립적으로 빌드하는 방식. 커널 소스를 참조(헤더·심볼)하되 커널 빌드에는 포함되지 않음.
# 반대로 in-tree 모듈은 커널 빌드 시 함께 컴파일되어 /lib/modules/ 에 설치됨.
make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 \
  M=drivers/hello_rust CONFIG_HELLO_RUST=m

# 2. RPi 5 전송
scp drivers/hello_rust/hello_rust.ko pi@192.168.219.102:~

# 3. RPi 5에서 로드
sudo insmod hello_rust.ko
dmesg | grep -E "Hello|Rust|hello_rust"

# 4. RPi 5에서 언로드
sudo rmmod hello_rust
dmesg | grep "Goodbye"
```

---

## 실행 결과

```
[  183.769457] hello_rust: loading out-of-tree module taints kernel.
[  183.769675] hello_rust: Hello from Rust! (init)
[  183.769678] hello_rust: Rust module loaded on Raspberry Pi 5
[  259.198255] hello_rust: Goodbye from Rust! (exit)
```

---

## 발생한 문제 및 해결 방법

| 문제 | 원인 | 해결 방법 |
|------|------|-----------|
| RPi 5 부팅 실패 (Phase 1 재작업 중) | `bcm2712-rpi-5-b.dtb` 교체 — RPi 펌웨어가 RPi 전용 패치 DTB를 요구 | DTB 교체 제거, 커널 이미지(`Image`)만 배포 |

---

## 학습 내용

- `module!{}` 매크로: 커널 모듈 메타데이터(이름, 저자, 라이선스 등)를 선언. C의 `MODULE_*` 매크로에 대응.
- `kernel::Module` 트레이트의 `init()`: C의 `module_init()` 함수에 대응. `Result<Self>`를 반환하여 에러 처리 내장.
- `Drop` 트레이트의 `drop()`: C의 `module_exit()` 함수에 대응. 모듈 언로드 시 자동 호출 — 명시적 해제 누락 불가.
- `loading out-of-tree module taints kernel`: 커널 트리 외부 모듈 로드 시 정상적으로 출력되는 경고. 동작에 영향 없음.
- RPi 5 배포 시 DTB는 교체하지 않는다. Raspberry Pi OS 원본 DTB에 RPi 전용 펌웨어 오버레이가 포함되어 있음.


---

## 다음 Phase 연계

- **Phase 3:** CVE-2023-52846 취약점 분석 — `cec-adap.c` Race Condition 발생 경로 및 패치 커밋 `9fe2816816a3` diff 분석
- **Phase 4:** Phase 3 분석 결과를 기반으로 Bug Injection 적용 및 KASAN 커널에서 UAF 실증
  - 필요 커널 옵션: `CONFIG_KASAN=y`, `CONFIG_KASAN_OUTLINE=y`
  - 대상 파일: `drivers/media/cec/core/cec-adap.c`
