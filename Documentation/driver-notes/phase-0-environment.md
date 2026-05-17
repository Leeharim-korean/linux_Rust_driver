# Phase 0 — 크로스 컴파일 환경 구축: Rust-for-Linux 개발 파이프라인 설계

---

## 프로젝트 배경 및 환경 구축 목적

임베디드 리눅스 환경에서 C 기반 디바이스 드라이버의 수동 메모리 관리 구조는 시스템 통합 검증 단계에서 간헐적인 커널 패닉을 유발하고, 원인 분석에만 수 주가 소요되는 구조적 리스크를 내포한다. 이를 해결하기 위한 **"Rust 언어"**의 커널 드라이버 적용 가능성을 실증하려면, Rust-for-Linux가 요구하는 툴체인 구성이 선행되어야 한다.

본 Phase에서는 **WSL2(Ubuntu 22.04) 호스트 환경에서 ARM64(Raspberry Pi 5) 타겟으로 크로스 컴파일**하는 개발 파이프라인을 구축한다. Rust-for-Linux는 LLVM 기반 빌드 시스템(`LLVM=1`)을 요구하며, `rustc`, `bindgen`, `clang` 각각의 최소 버전이 커널 소스의 `scripts/min-tool-version.sh`에 의해 강제된다. 이 버전 제약을 충족하지 못하면 `make rustavailable`이 실패하며 이후 모든 Phase가 차단된다.

```
# scripts/min-tool-version.sh
case "$1" in
binutils)
	echo 2.25.0
	;;
gcc)
	if [ "$ARCH" = parisc64 ]; then
		echo 12.0.0
	else
		echo 5.1.0
	fi
	;;
llvm)
	if [ "$SRCARCH" = s390 ]; then
		echo 15.0.0
	elif [ "$SRCARCH" = loongarch ]; then
		echo 18.0.0
	else
		echo 13.0.1
	fi
	;;
rustc)
	echo 1.78.0
	;;
bindgen)
	echo 0.65.1
	;;
*)
	echo "$1: unknown tool" >&2
	exit 1
	;;
esac
```

---

## 기술 스택 및 버전 정보

| 항목 | 버전 | 선택 근거 |
|------|------|-----------|
| `rustc` | 1.95.0 | 커널 요구 최소 버전(1.78.0) 충족, Rust-for-Linux Stable 채널 |
| `bindgen` | 0.72.1 | C 헤더 → Rust 바인딩 자동 생성 (커널 요구 최소: 0.65.1) |
| `clang` / `llvm` / `lld` | 14.0.0 | Rust-for-Linux LLVM=1 빌드에 필수 (최소: 13.0.1) |
| `aarch64-linux-gnu-gcc` | 11.4.0 | ARM64 어셈블러·링커 역할 (C 컴파일은 clang 담당) |
| 커널 브랜치 | rpi-6.12.y (6.12.83) | Raspberry Pi 공식 지원 브랜치, Rust 드라이버 기능 포함 |
| 호스트 OS | WSL2 Ubuntu 22.04.4 LTS | 리눅스 네이티브 빌드 환경 (Windows에서 커널 빌드 불가) |
| 타겟 보드 | Raspberry Pi 5 (BCM2712) | ARM64 실물 하드웨어 검증 환경 |

> **툴체인 버전:** `rustc`, `bindgen`, `clang`은 커널 소스 `Documentation/rust/quick-start.rst`에 명시된 버전을 준수하며, 독립 업그레이드 금지. 버전 불일치 시 `make rustavailable`에서 즉시 차단됨.

---

## 환경 구축 절차

```bash
# 1. 크로스 컴파일러 + LLVM 툴체인 설치
sudo apt install -y clang llvm lld aarch64-linux-gnu-gcc

# 2. Rust 툴체인 설치 (rustup 사용)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source ~/.cargo/env
rustup component add rust-src

# 3. bindgen 설치 (C 헤더 → Rust 바인딩 생성기)
cargo install --locked bindgen-cli

# 4. 커널 소스 클론 (전체 히스토리 필요 — git log 활용)
cd ~/01_Rust_for_linux_RPI_project
git config --global pack.threads 1   # WSL2 메모리 제한 대응
git clone -b rpi-6.12.y https://github.com/raspberrypi/linux

# 5. Rust 지원 가용성 검증
cd linux
make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 rustavailable
# 기대 출력: "Rust is available!"

# 6. GitHub fork 원격 저장소 설정
git remote set-url origin https://github.com/Leeharim-korean/linux_Rust_driver.git
git remote add upstream https://github.com/raspberrypi/linux.git
ssh-keygen -t ed25519 -C "hw051656@gmail.com"
git remote set-url origin git@github.com:Leeharim-korean/linux_Rust_driver.git
git checkout -b rust-cec-driver
git push -u origin rust-cec-driver
```

---

## 트러블슈팅 사례

### 이슈 1 — git clone 중단: WSL2 메모리 부족

**증상:**
```
fatal: fetch-pack: invalid index-pack output
```

**근본 원인 분석:**  
WSL2 기본 메모리 상한(3.7 GiB) 환경에서 대용량 git 오브젝트 팩을 멀티스레드로 처리할 시 OOM 발생. 커널 소스 히스토리(rpi-6.12.y)는 수 GiB 규모.

**해결 결정:**  
`pack.threads 1` 설정으로 병렬 처리 비활성화 → 메모리 피크 감소.  
`--depth=1` 얕은 클론은 `git log`로 커밋 히스토리 추적이 필요한 본 프로젝트 특성상 배제.

```bash
git config --global pack.threads 1
```

---

### 이슈 2 — GitHub 인증 실패

**증상:**
```
remote: Support for password authentication was removed.
```

**근본 원인 분석:**  
GitHub는 2021년 8월부터 HTTPS 패스워드 인증을 폐지. `origin` URL이 HTTPS 방식으로 설정되어 있었음.

**해결 결정:**  
SSH 키 페어 생성 후 GitHub 계정에 공개키 등록, `origin` URL을 SSH 방식(`git@github.com:`)으로 전환.

---

### 이슈 3 — SSH 최초 접속 시 호스트 키 검증 오류

**증상:**
```
Host key verification failed.
```

**해결 결정:**  
`-o StrictHostKeyChecking=accept-new` 옵션으로 최초 1회 자동 수락 후 `~/.ssh/known_hosts`에 등록.

---

## 기술적 의사결정 기준

| 결정 사항 | 선택 | 배제 대안 | 근거 |
|-----------|------|-----------|------|
| 클론 깊이 | 전체 히스토리 | `--depth=1` 얕은 클론 | CVE 분석을 위해 `git log`, `git diff` 사용 필수 |
| 인증 방식 | SSH 키 | HTTPS PAT | 자동화 파이프라인 확장 시 SSH가 적합 |
| 빌드 호스트 | WSL2 Ubuntu | Docker, native Linux | Windows 개발 환경에서 접근성 및 파일시스템 성능 균형 |

---

## 완료 기준 달성 확인

- [x] `make ARCH=arm64 CROSS_COMPILE=aarch64-linux-gnu- LLVM=1 rustavailable` → `Rust is available!`
- [x] GitHub fork(`Leeharim-korean/linux_Rust_driver`) 생성 및 `rust-cec-driver` 브랜치 push 완료

---

## 다음 Phase

- **Phase 1:** RPi 5 전용 커널 설정(`bcm2712_defconfig`) 적용 후 Rust 지원 활성화, 풀 커널 빌드 및 실물 보드 배포·부팅 검증
- **전제 조건:** 본 Phase에서 구성한 크로스 컴파일 파이프라인이 모든 이후 Phase의 빌드 기반이 됨

---

## 참고 자료

| 단계 | 참고 자료 | URL / 경로 |
|------|-----------|------------|
| Rust 툴체인 설치 (`rustup`) | Rust 공식 설치 가이드 | https://rustup.rs |
| Rust-for-Linux 툴체인 요구사항 | 커널 공식 Rust 시작 가이드 | `Documentation/rust/quick-start.rst` (커널 소스 내) |
| 최소 버전 확인 (`rustc`, `bindgen`, `clang`) | 커널 최소 버전 스크립트 | `scripts/min-tool-version.sh` (커널 소스 내) |
| `bindgen-cli` 설치 | bindgen 공식 문서 | https://rust-lang.github.io/rust-bindgen/ |
| 커널 소스 클론 | Raspberry Pi 공식 커널 저장소 | https://github.com/raspberrypi/linux |
| Rust-for-Linux 프로젝트 | Rust-for-Linux 공식 GitHub | https://github.com/Rust-for-Linux |
| LLVM/Clang 설치 (Ubuntu) | LLVM 공식 apt 저장소 | https://apt.llvm.org |
| GitHub SSH 키 인증 | GitHub 공식 SSH 인증 가이드 | https://docs.github.com/en/authentication/connecting-to-github-with-ssh |
