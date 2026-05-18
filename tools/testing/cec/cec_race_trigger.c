// SPDX-License-Identifier: GPL-2.0
/*
 * CVE-2023-52846 UAF Race Condition Trigger — KASAN 검증 모듈
 *
 * 목적:
 *   C 기반 커널 드라이버에서 발생하는 Use-After-Free(UAF) 결함이
 *   런타임 동적 분석(KASAN) 없이는 검출 불가능함을 실증한다.
 *   이를 통해 결함 발견 시점을 Runtime → Compile Time으로 앞당기는
 *   Shift-Left 전략의 필요성과 Rust 도입의 기술적 당위성을 입증한다.
 *
 * 재현 대상:
 *   CVE-2023-52846 — Linux HDMI-CEC 드라이버 cec_transmit_msg_fh()의
 *   Race Condition으로 인한 Use-After-Free. 패치 커밋: 9fe2816816a3.
 *
 * 취약 패턴:
 *   - cec_wait_timeout (workqueue): complete() 호출 후 data에 계속 접근
 *   - 메인 스레드: completed=true → cancel_delayed_work_sync 생략 → kfree(data)
 *   - work이 실행 중인 상태에서 kfree 호출 → USE-AFTER-FREE
 *
 * 재현 방법론:
 *   실제 CVE의 Race Window는 나노초 단위로 비결정적 재현이 어렵다.
 *   msleep(5)로 Race Window를 5ms로 확대하여 KASAN 검출을 결정적으로 달성.
 *   메모리 접근 패턴은 원본 CVE와 동일하게 유지.
 *
 * 사용법:
 *   sudo insmod cec_race_trigger.ko
 *   echo 1 | sudo tee /proc/cec_race_trigger
 *   dmesg | grep "BUG: KASAN"
 *
 * 검증 환경:
 *   커널: 6.12.83-v8-16k+ (KASAN OUTLINE 활성화)
 *   보드: Raspberry Pi 5 (ARM Cortex-A76, BCM2712)
 */
#include <linux/module.h>
#include <linux/kernel.h>
#include <linux/proc_fs.h>
#include <linux/slab.h>
#include <linux/completion.h>
#include <linux/workqueue.h>
#include <linux/delay.h>
#include <linux/uaccess.h>

MODULE_LICENSE("GPL");
MODULE_DESCRIPTION("CVE-2023-52846 UAF race condition trigger (KASAN test)");

/* cec_data의 필수 필드만 모방 */
struct cec_like_data {
	struct delayed_work work;
	struct completion c;
	bool completed;
	u8 payload[128];
};

/*
 * cec_wait_timeout() 취약 버전 재현.
 * complete() 이후 msleep으로 윈도우를 확보한 뒤 freed memory 접근.
 */
static void simulated_cec_wait_timeout(struct work_struct *work)
{
	struct cec_like_data *data =
		container_of(work, struct cec_like_data, work.work);

	/* cec_data_completed() 패턴: completed 설정 후 complete */
	data->completed = true;
	complete(&data->c);

	/*
	 * 레이스 윈도우 확대 (실제 CVE는 나노초, 여기서는 5ms):
	 * 메인 스레드가 kfree(data)를 호출할 충분한 시간을 준다.
	 */
	msleep(5);

	/*
	 * UAF: kfree된 메모리에 접근.
	 * 실제 CVE에서는 workqueue 인프라가 work.work.entry를 접근.
	 * KASAN이 이 시점에서 "use-after-free" 리포트를 출력해야 한다.
	 */
	pr_info("cec_race: [BUG] accessing freed data->completed = %d\n",
		data->completed);
}

static void trigger_race_once(void)
{
	struct cec_like_data *data;

	data = kzalloc(sizeof(*data), GFP_KERNEL);
	if (!data)
		return;

	init_completion(&data->c);
	INIT_DELAYED_WORK(&data->work, simulated_cec_wait_timeout);

	/* cec_wait_timeout을 1 jiffie 후 실행 */
	schedule_delayed_work(&data->work, 1);

	/* 블로킹 대기 (cec_transmit_msg_fh의 wait_for_completion_killable) */
	wait_for_completion_killable(&data->c);

	/*
	 * CVE-2023-52846 취약 패턴:
	 * completed=true이므로 cancel_delayed_work_sync()를 건너뜀.
	 * work 함수는 아직 msleep(5) 중이지만 kfree를 호출한다.
	 */
	if (!data->completed)
		cancel_delayed_work_sync(&data->work);

	kfree(data); /* work이 msleep 중일 때 해제 → UAF */
}

static ssize_t trigger_write(struct file *file, const char __user *ubuf,
			     size_t count, loff_t *ppos)
{
	int i;

	pr_info("cec_race: starting UAF trigger (CVE-2023-52846 pattern)\n");
	for (i = 0; i < 5; i++) {
		trigger_race_once();
		pr_info("cec_race: attempt %d done\n", i + 1);
		msleep(10);
	}
	pr_info("cec_race: trigger complete. Check dmesg for KASAN report.\n");
	return count;
}

static const struct proc_ops trigger_proc_ops = {
	.proc_write = trigger_write,
};

static struct proc_dir_entry *proc_entry;

static int __init cec_race_trigger_init(void)
{
	proc_entry =
		proc_create("cec_race_trigger", 0222, NULL, &trigger_proc_ops);
	if (!proc_entry)
		return -ENOMEM;

	pr_info("cec_race: module loaded.\n");
	pr_info("cec_race: trigger with: echo 1 | sudo tee /proc/cec_race_trigger\n");
	return 0;
}

static void __exit cec_race_trigger_exit(void)
{
	proc_remove(proc_entry);
	pr_info("cec_race: module unloaded.\n");
}

module_init(cec_race_trigger_init);
module_exit(cec_race_trigger_exit);
