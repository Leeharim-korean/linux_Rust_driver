# Phase 4 — Bug Injection + KASAN: CVE-2023-52846 UAF 취약점 실증

---

## 프로젝트 배경 및 기술적 목적

C 기반 임베디드 드라이버 개발 환경에서 시스템 통합 검증 단계에 **간헐적**으로 발생하는 커널 패닉은 원인 분석에만 수 주가 소요되어 양산 일정에 직접적인 지연 리스크를 초래한다. 특히 다중 스레드 환경에서 메모리를 공유하는 구조에서 경쟁 상태(Race Condition)로 인한 Use-After-Free(UAF) 결함은 **C 언어의 수동 메모리 관리 구조**상 정적 분석 도구만으로는 사전 검출이 불가능하며, 실차 시험 단계에서 치명적인 이슈로 이어지는 악순환을 만든다.

본 Phase의 목적은 **CVE-2023-52846을 실물 ARM64 하드웨어(Raspberry Pi 5) 위에서 KASAN(Kernel Address Sanitizer)을 통해 동적으로 실증**하는 것이다. 이는 두 가지 목표를 동시에 달성한다:

1. **C 언어의 구조적 한계 실증**: 런타임 동적 분석 없이는 검출 불가능한 UAF를 KASAN으로 확인
2. **Rust 도입 당위성**: Phase 5의 Rust 구현이 동일 취약점을 **컴파일 타임**에 원천 차단함을 비교 검증하기 위한 기준선(baseline) 수립

단순한 코드 분석에 그치지 않고, C 기반 드라이버의 취약 시나리오를 직접 재현함으로써 **결함 발견 시점을 Runtime -> Compile Time으로 앞당기는 Shift-Left 전략**의 필요성을 실물 데이터로 뒷받침한다.

---

## Rust가 이 문제를 컴파일 타임에 차단하는 이유

C의 취약 패턴과 Rust의 구조적 해결책을 선제적으로 비교하여, 본 Phase의 검증이 갖는 기술적 의미를 명확히 한다.

**C의 취약 패턴:**

```c
struct cec_data {
    bool completed;   // 비원자적 — 다중 CPU 환경에서 원자성 미보장
};

// CPU 0: cec_transmit_msg_fh
if (!data->completed)              // completed=true -> 조건 미충족 -> cancel 생략
    cancel_delayed_work_sync();
kfree(data);                       // work이 아직 실행 중임에도 해제 -> UAF

// CPU 1: cec_wait_timeout (workqueue)
data->completed = true;
complete(&data->c);                // CPU 0을 깨우는 시점에 work는 아직 실행 중
```

**Rust의 컴파일 타임 차단:**

| C의 구조적 문제 | Rust의 해결 | 차단 시점 |
|---|---|---|
| `bool completed` — 비원자적 공유 | `AtomicBool` — 일반 `bool`은 `Sync` 트레이트 위반으로 컴파일 거부 | 컴파일 타임 |
| `kfree(data)` — 참조 중 해제 가능 | `Arc<CecData>` — 마지막 참조 소멸 시에만 해제. 타이머가 보유 중이면 불가 | 컴파일 타임 |
| 조건부 `cancel_delayed_work_sync()` — 누락 가능 | `Drop` 트레이트 — 소유자 소멸 시 무조건 실행, 빠뜨리면 컴파일 에러 | 컴파일 타임 |

---

## 취약점 재현 설계

### Bug Injection 전략

CVE 패치 커밋(`9fe2816816a3`)을 직접 되돌리는 `git revert` 방식은 이후 커밋으로 인한 컨텍스트 불일치로 실패했다. 대신 **취약 버전 소스 파일을 별도 생성하고 Makefile 스위칭**으로 교체하는 방식을 채택했다.

```
drivers/media/cec/core/
├── cec-adap.c         ← 안전 버전 (패치 적용, 기본 빌드 대상)
└── cec-adap_error.c   ← 취약 버전 (Bug Injection — Makefile 전환으로 활성화)
```

**취약 코드 (`cec-adap_error.c`, 966~968번 줄):**

```c
// cec-adap.c (안전 — 패치 후)
err = wait_for_completion_killable(&data->c);
cancel_delayed_work_sync(&data->work);      // 무조건 호출 — Race Window 없음

// cec-adap_error.c (취약 — Bug Injection)
err = wait_for_completion_killable(&data->c);
if (!data->completed)                        // 조건부 호출 — Race Window 형성
    cancel_delayed_work_sync(&data->work);
```

**Makefile 전환 (한 줄 변경):**

```makefile
# 취약 버전 활성화
cec-objs := cec-core.o cec-adap_error.o cec-api.o

# 안전 버전 복원 시
# cec-objs := cec-core.o cec-adap.o cec-api.o
```

### KASAN 커널 빌드

```bash
# KASAN 활성화 (OUTLINE 모드: 오버헤드 최소화, RPi 5 적합)
./scripts/config --enable CONFIG_KASAN
./scripts/config --set-val CONFIG_KASAN_OUTLINE y
make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 olddefconfig
make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 -j$(nproc)
```

> **KASAN OUTLINE:** INLINE 모드 대비 코드 크기와 런타임 오버헤드를 줄여 리소스가 제한된 RPi 5(ARM Cortex-A76, 4GB RAM) 환경에 적합. KASAN 활성화 시 이미지 크기: 24MB -> 39MB.

### 안전 버전 백업 (원복 경로 확보)

KASAN 빌드가 기존 이미지를 덮어쓰므로, 배포 전 원복 가능한 안전망을 먼저 확보한다.

```bash
BACKUP_DIR=~/01_Rust_for_linux_RPI_project/backup/safe-kernel
mkdir -p $BACKUP_DIR
cp ~/01_Rust_for_linux_RPI_project/linux/arch/arm64/boot/Image $BACKUP_DIR/Image.safe
cp ~/01_Rust_for_linux_RPI_project/linux/.config              $BACKUP_DIR/.config.safe
cp -r /tmp/rpi-modules/lib/modules/6.12.83-v8-16k+            $BACKUP_DIR/modules-6.12.83-v8-16k+
```

---

## UAF 취약점 실증

### 재현 방법론

CEC 취약점은 타이머 콜백(workqueue)과 전송 완료 경로 사이의 Race Condition에 의해 발생한다. 실물 HDMI CEC 하드웨어 없이도 동일한 Race Condition 패턴을 재현하기 위해, **CVE 취약 코드 패턴을 그대로 모방한 커널 모듈 `cec_race_trigger.ko`를 설계**했다.

Race Window 인위적 확대 전략: 실제 CVE의 Race Window는 나노초 단위로 비결정적 재현이 어렵다. `msleep(5)`를 통해 Race Window를 5ms로 확대하여 100% 재현율을 확보했다. 이는 실제 CVE와 동일한 메모리 접근 패턴을 유지하면서 재현 가능성을 높인 검증 방법론이다.

```c
static void simulated_cec_wait_timeout(struct work_struct *work)
{
    struct cec_like_data *data = container_of(work, ...);

    /* cec_data_completed() : completed 설정 -> complete() 호출 */
    data->completed = true;
    complete(&data->c);          // 메인 스레드 깨움

    msleep(5);                   // Race Window 확대 (CVE에서는 나노초 단위)

    (void)data->completed;       // UAF: kfree 이후 freed memory 접근
}

static void trigger_race_once(void)
{
    struct cec_like_data *data = kzalloc(sizeof(*data), GFP_KERNEL);
    // ...
    schedule_delayed_work(&data->work, 1);
    wait_for_completion_killable(&data->c);

    /* CVE-2023-52846 취약 패턴: completed=true -> cancel 생략 */
    if (!data->completed)
        cancel_delayed_work_sync(&data->work);   // 항상 건너뜀

    kfree(data);   // work이 msleep 중인 상태에서 해제 -> UAF 발생
}
```

**CVE 원본과 cec_race_trigger의 대응 관계:**

| CVE-2023-52846 원본 | cec_race_trigger.ko |
|---|---|
| `cec_wait_timeout()` | `simulated_cec_wait_timeout()` |
| `data->completed = true; complete()` | 동일 |
| 워크큐 후처리 중 `data` 접근 (UAF 지점) | `msleep(5)` 후 `data->completed` 접근 (명시적 UAF) |
| `kfree(data)` (cancel 생략 후) | 동일 |

### 트리거 방법

```bash
# 모듈 로드
sudo insmod /tmp/cec_race_trigger.ko

# UAF 트리거 (5회 반복)
echo 1 | sudo tee /proc/cec_race_trigger

# KASAN 리포트 확인
dmesg | grep -A 60 "BUG: KASAN"
```

---

## 검증 결과: KASAN UAF 리포트

```
[ 5980.786368] ==================================================================
[ 5980.786378] BUG: KASAN: slab-use-after-free in simulated_cec_wait_timeout+0x4c/0x70 [cec_race_trigger]
[ 5980.786397] Read of size 1 at addr ffff800004e1b478 by task kworker/2:1/8461

[ 5980.786415] CPU: 2 UID: 0 PID: 8461 Comm: kworker/2:1 Tainted: G O 6.12.83-v8-16k+ #2
[ 5980.786434] Workqueue: events simulated_cec_wait_timeout [cec_race_trigger]
[ 5980.786446] Call trace:
[ 5980.786501]  simulated_cec_wait_timeout+0x4c/0x70 [cec_race_trigger]
[ 5980.786509]  process_one_work+0x2b0/0x6c8

[ 5980.786558] Allocated by task 22594:
[ 5980.786604]  trigger_write+0xb8/0x1b0 [cec_race_trigger]

[ 5980.786714] Freed by task 22594:
[ 5980.786758]  trigger_write+0x80/0x1b0 [cec_race_trigger]

[ 5980.786966] The buggy address belongs to the object at ffff800004e1b400
                which belongs to the cache kmalloc-256 of size 256
[ 5980.786976] The buggy address is located 120 bytes inside of
                freed 256-byte region [ffff800004e1b400, ffff800004e1b500)
==================================================================
```

### 리포트 해석

| 항목 | 내용 | 의미 |
|------|------|------|
| 버그 유형 | `slab-use-after-free` | 힙 메모리 해제 후 접근 |
| UAF 접근 위치 | `simulated_cec_wait_timeout+0x4c` | `data->completed` (1바이트) |
| 할당 경로 | `trigger_write -> kzalloc` | `cec_transmit_msg_fh`의 `kzalloc(data)` 대응 |
| 해제 경로 | `trigger_write -> kfree` | `cancel` 생략 직후 `kfree(data)` |
| UAF 발생 | workqueue의 `msleep` 후 접근 | workqueue 후처리 중 freed memory 접근 |
| 재현율 | **5/5회 (100%)** | Race Window 5ms 확대로 결정적 재현 달성 |

### Shift-Left 실증 결과

| 지표 | C 드라이버 (취약) | Rust 드라이버 (Phase 5 목표) |
|------|---|---|
| 결함 발견 시점 | **Runtime** — KASAN 동적 분석으로만 검출 가능 | **Compile Time** — 컴파일 거부로 코드 작성 자체 불가 |
| 검출 방법 | KASAN (동적 메모리 분석 도구) | Rust 컴파일러 소유권 검사 |
| 재현 조건 | Race Condition (비결정적, 환경 의존) | 해당 없음 (컴파일 단계에서 원천 차단) |
| 디버깅 비용 | 수 주 (원인 파악만으로) | 0 (컴파일 오류 메시지로 즉시 식별) |

---

## 트러블슈팅 사례

### 이슈 1 — KASAN 커널 부팅 후 전체 모듈 로드 실패

**증상:**
```
module ipv6: .gnu.linkonce.this_module section size must match the kernel's built struct module size
module x_tables: .gnu.linkonce.this_module section size must match the kernel's built struct module size
[FAILED] Failed to start systemd-modules-load.service
```

**근본 원인 분석:**  
KASAN 활성화 시 커널 내부 `struct module` 크기가 변경된다. 커널 이미지만 교체하고 기존 모듈을 그대로 사용하면 구조체 크기 불일치로 모든 모듈 로드가 차단된다.

**해결 결정:**  
KASAN 커널 이미지와 KASAN 커널용 모듈을 반드시 동일 빌드에서 생성하여 세트로 배포.

```bash
# 커널 이미지 + 모듈 세트 배포 (반드시 동일 빌드)
make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 \
  INSTALL_MOD_PATH=/tmp/rpi-kasan-modules modules_install

scp arch/arm64/boot/Image pi@<ip>:/tmp/kernel_kasan.img
scp -r /tmp/rpi-kasan-modules/lib/modules/6.12.83-v8-16k+/kernel \
  pi@<ip>:/tmp/kasan-kernel-new

# RPi에서 모듈 교체 후 재부팅
sudo rm -rf /lib/modules/6.12.83-v8-16k+/kernel
sudo mv /tmp/kasan-kernel-new /lib/modules/6.12.83-v8-16k+/kernel
```

> 커널 이미지 교체 시 모듈도 반드시 함께 교체. 배포 순서: 이미지 전송 -> 모듈 설치 -> 재부팅.

---

### 이슈 2 — 모듈 로드 실패로 네트워크 미활성화 -> SSH 불가

**증상:**
```
ssh: connect to host 192.168.219.102 port 22: No route to host
```

**근본 원인 분석:**  
`ipv6`, `x_tables` 등 네트워크 관련 모듈 로드 실패로 네트워크 인터페이스 미활성화. IP가 `127.0.0.1`로 표시되며 외부 접속 불가.

**해결 결정:**  
RPi 5 물리 터미널에서 직접 `config.txt` 수정 후 안전 커널로 복구 부팅.

```bash
sudo sed -i 's/^kernel=kernel_kasan.img/#kernel=kernel_kasan.img/' /boot/firmware/config.txt
sudo reboot
```

이후 WiFi 초기화 실패로 이더넷 케이블로 우회 -> `eth0` UP -> IP `124.60.255.121` 할당 -> SSH 재접속 성공.

---

### 이슈 3 — dtoverlay 오설정으로 vc4 로드 실패 + CLI 부팅

**증상:**
```
modprobe: ERROR: could not insert 'vc4': Invalid argument
Physical Address: f.f.f.f   (CEC 어댑터 — 연결 없음)
```

**근본 원인 분석:**  
`/boot/firmware/config.txt`에 RPi 4용 `dtoverlay=vc4-kms-v3d`가 설정되어 있었음. RPi 5는 `vc4-kms-v3d-pi5`를 사용해야 하며, 잘못된 오버레이로 인해 Device Tree에 VideoCore IV 노드가 부재 -> `vc4` 드라이버 probe 실패.

**문제점:** vc4 미초기화 -> GPU/HDMI 미활성화 -> lightdm 미시작 -> CLI 부팅.  
**해결 계획 (Phase 완료 후):** `dtoverlay=vc4-kms-v3d-pi5`로 수정 후 재부팅.

---

### 이슈 4 — 모듈 `.ko.xz` 파일 전송 손상

**증상:**
```
xz: /lib/modules/.../cec.ko.xz: File format not recognized
modprobe: ERROR: could not insert 'cec': Invalid argument
```

**근본 원인 분석:**  
`scp -r`로 모듈 디렉토리 전송 시 `build/` 심볼릭 링크를 따라가며 커널 소스 전체(수 GB)를 RPi `/tmp/`에 복사 시도. 저장 용량 초과로 `kernel/` 디렉토리의 `.ko.xz` 파일이 truncated(0바이트) 상태로 저장됨.

**해결 결정:**  
`build/` 심볼릭 링크 제외, `kernel/` 서브디렉토리만 별도 전송.

```bash
scp -r /tmp/rpi-kasan-modules/lib/modules/6.12.83-v8-16k+/kernel \
  pi@<ip>:/tmp/kasan-kernel-new
```

> `scp -r`로 심볼릭 링크 포함 디렉토리 전송 시 반드시 `rsync --exclude='build'` 또는 서브디렉토리 단위 분리 전송.

---

### 이슈 5 — CEC 하드웨어 미지원 환경에서의 우회 전략

**상황:**  
연결된 모니터가 HDMI CEC 미지원(`Physical Address: f.f.f.f`). CEC 버스 미형성으로 `cec_transmit_msg_fh()` 블로킹 전송 경로 진입 불가 -> 실물 CEC로 Race Condition 트리거 불가.

**우회 전략 1 — vivid 가상 CEC 어댑터:**  
`CONFIG_VIDEO_VIVID_CEC=y`로 vivid 재빌드 시도. Kconfig에서 `VIDEO_VIVID_CEC`(bool)가 `CEC_CORE=m`을 `select`하는 구조적 충돌로 `auto.conf`에 미반영. Makefile 직접 수정으로 해결.

```makefile
# Kconfig 우회: 빌드 강제 포함
vivid-objs += vivid-cec.o
ccflags-y  += -DCONFIG_VIDEO_VIVID_CEC
```

vivid 가상 어댑터(`/dev/cec2`, `/dev/cec3`) 생성 성공. 그러나 가상 버스에서의 Race Window가 나노초 단위로 결정적 재현 불가 (2,000회 시도, 미발생).

**우회 전략 2 — cec_race_trigger.ko (최종 채택):**  
CVE 취약 패턴을 그대로 모방한 커널 모듈을 별도 설계. `msleep(5)`로 Race Window를 5ms로 확대하여 KASAN 리포트 100% 재현 성공.


---

## 다음 Phase

- **Phase 5:** 동일 CEC 드라이버를 Rust로 구현. `AtomicBool`, `Arc<CecData>`, `Drop` 트레이트를 활용하여 본 Phase에서 실증한 UAF 취약점 클래스가 컴파일 타임에 원천 차단됨을 증명. C vs Rust 1:1 비교 설계 수행.
- **Phase 6:** KCOV 커버리지 도구로 C 드라이버와 Rust 드라이버의 오류 경로 커버리지를 수치화하여 Shift-Left 효과를 정량적으로 입증.

---

## 참고 자료

| 항목 | 참고 자료 | URL / 경로 |
|------|-----------|------------|
| CVE-2023-52846 상세 정보 | NVD 취약점 데이터베이스 | https://nvd.nist.gov/vuln/detail/CVE-2023-52846 |
| 패치 커밋 `9fe2816816a3` | Linux 커널 공식 git | https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git/commit/?id=9fe2816816a3 |
| KASAN 사용법 및 리포트 해석 | 커널 공식 문서 | `Documentation/dev-tools/kasan.rst` (커널 소스 내) |
| Kernel Workqueue API | 커널 공식 문서 | `Documentation/core-api/workqueue.rst` (커널 소스 내) |
| `cancel_delayed_work_sync` | 커널 API 문서 | `Documentation/core-api/workqueue.rst` (커널 소스 내) |
| KASAN OUTLINE vs INLINE | 커널 KASAN 문서 | `Documentation/dev-tools/kasan.rst` §Instrumentation (커널 소스 내) |
| 커널 모듈 개발 기초 | 커널 모듈 문서 | `Documentation/kbuild/modules.rst` (커널 소스 내) |
| procfs 인터페이스 (`proc_create`) | 커널 API | `Documentation/filesystems/proc.rst` (커널 소스 내) |
| UAF 트리거 모듈 소스 | 본 저장소 | `tools/testing/cec/cec_race_trigger.c` |
| Bug Injection 대상 파일 | 본 저장소 | `drivers/media/cec/core/cec-adap_error.c` |
