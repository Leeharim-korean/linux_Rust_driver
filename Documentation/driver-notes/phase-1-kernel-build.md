# Phase 1 — Rust 지원 커널 빌드 및 RPi 5 배포 검증


---

## 목적 및 기술적 배경

C 기반 드라이버의 메모리 안전성 한계를 Rust로 개선하기 위한 사전 작업으로, Rust-for-Linux 커널이 실물 ARM64 하드웨어(Raspberry Pi 5)에서 정상 부팅됨을 검증해야 한다. 이 단계가 확립되어야 이후 Phase에서 KASAN 커널 배포, Rust 드라이버 모듈 삽입/제거(`insmod`/`rmmod`) 등 모든 실하드웨어 검증이 가능하다.

Raspberry Pi 5는 Broadcom BCM2712(ARM Cortex-A76 기반) SoC를 탑재하며, 공식 RPi 커널 브랜치 `rpi-6.12.y`가 이를 지원한다. 커스텀 커널 빌드 시 기존 RPi OS 커널(`6.12.75+rpt-rpi-2712`)과 병행 부팅 구조를 유지하여, 배포 실패 시 즉시 원복이 가능한 안전망을 확보한다.

---

## 환경 정보

| 항목 | 버전/값 |
|------|---------|
| 커널 브랜치 | rpi-6.12.y |
| 빌드 커널 버전 | 6.12.83-v8-16k+ |
| 기존 RPi OS 커널 (유지) | 6.12.75+rpt-rpi-2712 |
| 타겟 보드 | Raspberry Pi 5 (BCM2712, Cortex-A76) |
| 페이지 크기 | 16KB (`-16k+` 접미사) |
| 배포 방법 | SSH (scp / rsync) |
  
> - '유지': 커스텀 커널을 별도 파일명(`kernel_rust.img`)으로 추가하고 `config.txt`에 `kernel=kernel_rust.img` 지정. 기존 커널 파일은 삭제하지 않으므로 해당 줄 제거 시 즉시 원복 가능  
> - 16KB 페이지: RPi 5 기본값. 페이지는 OS가 물리 메모리를 관리하는 최소 단위이며 일반 ARM은 4KB이나 RPi 5는 16KB 사용. 페이지 크기는 커널 빌드 시 결정되며, **4KB용 모듈은 16KB 커널에 로드 불가**
---

## 빌드 및 배포 절차

```bash
# 1. RPi 5 전용 기본 설정 적용 (bcm2712_defconfig)
make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 bcm2712_defconfig

# 2. Rust 지원 활성화
# CONFIG_MODVERSIONS는 Rust와 상호 배타, 심볼 생성에 문제 (6.12 기준 미지원, 6.13에서 해결 예정)
./scripts/config --disable CONFIG_MODVERSIONS
./scripts/config --enable CONFIG_RUST
make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 olddefconfig

# 3. 풀 커널 빌드 (빌드 결과: arch/arm64/boot/Image, 24MB)
make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 -j$(nproc) 2>&1 | tee build.log

# 4. 모듈 설치 준비
make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 \
  INSTALL_MOD_PATH=/tmp/rpi-modules modules_install

# 5. RPi 5 배포 (Image + DTB + 모듈 세트 필수)
# scp: 단일 파일 전송 — 커널 이미지, DTB 각각 복사
scp arch/arm64/boot/Image pi@192.168.219.102:/tmp/kernel_rust.img
scp arch/arm64/boot/dts/broadcom/bcm2712-rpi-5-b.dtb pi@192.168.219.102:/tmp/
# rsync: 디렉토리 전체 전송에 최적화 — 수백 개의 .ko.xz 모듈 파일을 속성(권한·링크·타임스탬프) 보존하며 전송
# -a(속성 보존+재귀) -v(진행 출력) -z(압축 전송)
rsync -avz /tmp/rpi-modules/lib/modules/ pi@192.168.219.102:/tmp/modules/

# 6. RPi 5에서 배치 및 부팅 설정
#    config.txt의 kernel= 항목으로 커스텀 커널 지정 (기존 커널 유지)
sudo cp /tmp/kernel_rust.img /boot/firmware/kernel_rust.img
sudo cp /tmp/bcm2712-rpi-5-b.dtb /boot/firmware/bcm2712-rpi-5-b.dtb
sudo cp -r /tmp/modules/* /lib/modules/
echo "kernel=kernel_rust.img" | sudo tee -a /boot/firmware/config.txt
sudo reboot
```

---

## 기술적 의사결정 기준

### CONFIG_MODVERSIONS 비활성화

`CONFIG_MODVERSIONS`는 커널 모듈 심볼에 CRC 버전 체크를 추가하는 기능으로, 내부적으로 `genksyms` 도구가 소스 파싱을 담당한다. 현재 `genksyms`는 C 문법만 파싱 가능하여 Rust 심볼을 처리할 수 없다. 이로 인해 `CONFIG_MODVERSIONS=y` 상태에서는 `CONFIG_RUST`를 활성화할 수 없으며, 커널 6.13부터 Rust MODVERSIONS 지원이 추가될 예정이다.

### KASAN 모드: OUTLINE vs INLINE

| 모드 | 특징 | 선택 |
|------|------|------|
| OUTLINE | 섀도 메모리 접근을 함수 호출로 처리, 코드 크기↓, 오버헤드↓ | **선택** |
| INLINE | 섀도 접근 코드를 직접 삽입, 탐지 속도↑, 코드 크기↑ | 배제 |

RPi 5(ARM Cortex-A76, 4GB RAM)의 리소스 제약과 실보드 검증 목적을 고려하여 OUTLINE 모드를 채택.

### 병행 부팅 전략

`/boot/firmware/config.txt`의 `kernel=` 항목으로 부팅 커널을 지정하는 RPi 펌웨어 특성을 활용하여, 기존 안정 커널과 커스텀 빌드 커널을 동시에 유지한다. 이는 이후 KASAN 빌드, Bug Injection 커널 등 다수의 커널 이미지를 안전하게 전환할 수 있는 기반이 된다.

```
# /boot/firmware/config.txt
kernel=kernel_rust.img     ← 커스텀 커널 (활성)
# 기존 kernel8.img는 삭제하지 않음 → 복구 가능
```

---

## 트러블슈팅 사례

### 이슈 1 — CONFIG_RUST 활성화 불가

**증상:** `make olddefconfig` 후에도 `CONFIG_RUST=n`으로 남음

**근본 원인:** `CONFIG_MODVERSIONS=y` 상태에서 Kconfig 의존성 충돌. `genksyms`의 Rust 파싱 미지원으로 인한 커널 6.12 구조적 제약.

**해결:** `CONFIG_MODVERSIONS` 명시적 비활성화 후 재적용.

```bash
./scripts/config --disable CONFIG_MODVERSIONS
./scripts/config --enable CONFIG_RUST
make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 olddefconfig
```


---

### 이슈 2 — /proc/config.gz 미존재

**증상:** 커널 설정 확인 시 `zcat /proc/config.gz` 실패

**근본 원인:** `CONFIG_IKCONFIG_PROC` 비활성화 상태.

**대안:** `/proc/kallsyms`로 Rust 심볼 로드 여부 확인.
```bash
grep -c "rust" /proc/kallsyms   # 118개 확인
```

---

## 검증 결과

| 검증 항목 | 결과 |
|-----------|------|
| 커스텀 커널 부팅 | `6.12.83-v8-16k+` 정상 부팅 |
| Rust 심볼 로드 | `/proc/kallsyms` grep 결과 118개 확인 |
| 기존 커널 병행 유지 | `kernel8.img` 미삭제, 복구 가능 상태 |
| SSH 원격 접속 | `pi@192.168.219.102` 접속 정상 |


---

## 다음 Phase 연계

- **Phase 2:** `drivers/hello_rust/` 구조의 최소 Rust 커널 모듈 작성, `insmod`/`rmmod` 검증

---

## 참고 자료

| 항목 | 참고 자료 | URL / 경로 |
|------|-----------|------------|
| RPi 5 커널 빌드 공식 가이드 | Raspberry Pi 공식 문서 | https://www.raspberrypi.com/documentation/computers/linux_kernel.html |
| `config.txt` 커널 지정 (`kernel=`) | RPi 펌웨어 설정 참고 | https://www.raspberrypi.com/documentation/computers/config_txt.html |
| `bcm2712_defconfig` | RPi 공식 커널 저장소 | https://github.com/raspberrypi/linux |
| Rust-for-Linux 빌드 요구사항 | 커널 Rust 시작 가이드 | `Documentation/rust/quick-start.rst` (커널 소스 내) |
| CONFIG_MODVERSIONS / Rust 충돌 | 커널 Kconfig 제약 문서 | `Documentation/kbuild/kconfig-language.rst` (커널 소스 내) |
| ARM64 크로스 컴파일 | Linaro 크로스 툴체인 | https://www.linaro.org/downloads/ |
