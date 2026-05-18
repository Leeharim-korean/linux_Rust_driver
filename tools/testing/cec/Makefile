obj-m := cec_race_trigger.o

# WSL 크로스 컴파일 기본값 (환경변수로 override 가능)
KDIR  ?= /home/leeharim_korean/01_Rust_for_linux_RPI_project/linux
ARCH  ?= arm64
CROSS ?= aarch64-linux-gnu-
PWD   := $(shell pwd)

all:
	make -C $(KDIR) M=$(PWD) ARCH=$(ARCH) CROSS_COMPILE=$(CROSS) LLVM=1 modules

# RPi 5 네이티브 빌드: make native
native:
	make -C /lib/modules/$(shell uname -r)/build M=$(PWD) modules

clean:
	make -C $(KDIR) M=$(PWD) ARCH=$(ARCH) CROSS_COMPILE=$(CROSS) LLVM=1 clean
