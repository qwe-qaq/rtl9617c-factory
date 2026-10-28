# rtl9617c_factory

RTL9617C 平台 Factory 镜像生成工具。从设备备份的 U-Boot env 分区与 `ubi_device` 容器镜像中提取产线身份数据与校准数据，输出统一的 8 MiB Factory bin。该 bin 由 U-Boot 写入 UBI static 卷 `factory`。

支持机型：

| model | 设备 | 数据源 |
| --- | --- | --- |
| `tyfza1` | 天邑 TYFZA1（Wi-Fi + 下行光 + FPGA） | `ubi_Config` 卷内 `config_hs.xml` / `config.xml` / `rtkbosa_k.bin` / `rtkoltbosa_k.bin` / `xtemp.cal`，`ubi_fpga` 卷 |
| `pt1500-h10` | 友华 PT1500-H10（无 Wi-Fi / 无下行光） | U-Boot env `kp_*` keyparam 变量 |

## 构建

```sh
cargo build --release --locked
```

## 用法

```sh
rtl9617c-factory build --model <tyfza1|pt1500-h10> \
    --env <mtd1_env.bin> [--env2 <mtd2_env2.bin>] \
    --device <mtd4_ubi_device.bin> -o <factory.bin>

rtl9617c-factory dump  --model <...> --env <...> [--env2 <...>] --device <...>
```

`build` 输出 8 MiB bin 并打印各段摘要（偏移 / 大小 / CRC）；`dump` 打印提取到的全部字段。

## Factory bin 规范

- 总大小 `0x800000`（8 MiB），多字节整数一律小端（LE），除注明 `be` 外。
- 未使用区域填 `0xFF`；ASCII 字符串以 NUL（`0x00`）结尾，其后仍为 `0xFF`。
- 空字段：目录条目 `flags` 为 0 或条目不存在，对应区域保持 `0xFF`。

### Header（`0x000000` – `0x000400`）

| 偏移 | 大小 | 字段 | 说明 |
| --- | --- | --- | --- |
| `0x000` | 8 | `magic` | `"RTLFACT1"` |
| `0x008` | 4 | `version` | 镜像格式版本，当前 `1` |
| `0x00C` | 4 | `image_size` | `0x800000` |
| `0x010` | 4 | `model` | `1`=tyfza1，`2`=pt1500-h10 |
| `0x014` | 4 | `image_crc` | 整镜像 CRC32，计算时本字段置 0 |
| `0x018` | 8 | `generation` | 生成时间（Unix 秒） |
| `0x020` | 4 | `table_crc` | 目录区 CRC32（`0x200`–`0x3FC`） |
| `0x024` | 4 | `header_crc` | 见 CRC 规则 |
| `0x028` | `0x1D8` | 保留 | `0xFF` |

### 目录（`0x000200` – `0x000400`）

32 个条目，每条 16 字节，按 tag 升序排列：

| 偏移 | 大小 | 字段 |
| --- | --- | --- |
| `+0` | 2 | `tag`（LE u16） |
| `+2` | 2 | `flags`：bit0 = present |
| `+4` | 4 | `offset`（LE u32，镜像内绝对偏移） |
| `+8` | 4 | `size`（LE u32，实际数据长度，不含 pad） |
| `+12` | 4 | `crc32`（数据区 CRC32，zlib 多项式） |

空条目保持 `0xFF`（`tag == 0xFFFF` 即视为不存在）。

### 字段区（固定偏移）

只收录产线烧录的硬件身份与校准数据。LOID/PLOAM/账户、产品型号、省份、默认 SSID 等属于上层软件自定义数据，不入 Factory bin（tag `0x0012`–`0x00FF` 预留给未来新的硬件产线字段）。

| 偏移 | 容量 | Tag | 字段 | 类型 | tyfza1 来源 | pt1500-h10 来源 |
| --- | --- | --- | --- | --- | --- | --- |
| `0x001000` | `0x10` | `0x0001` | `BASE_MAC` | 6B MAC + 4B count | `config_hs.xml` `ELAN_MAC_ADDR`，count 固定 3 | env `kp_ucaBaseMacAddr` + `kp_ulNumMacAddrs` |
| `0x001010` | `0x06` | `0x0002` | `EPON_MAC` | 6B MAC | `config.xml` `EPON_LLID_TBL.macAddr`（无则空） | 等于 BASE_MAC |
| `0x001020` | `0x10` | `0x0010` | `PON_SN` | ASCII | `GPON_SN`（12 字符） | env `kp_szXponSN`（12 字符） |
| `0x001030` | `0x20` | `0x0011` | `DEVICE_SN` | ASCII | 无（空） | env `kp_szSerialNumber` |
| `0x002000` | `0x1000` | `0x0100` | `WIFI_CAL_RADIO0` | 二进制（TYFC0003） | 5G `rtl8852ce` 校准组装 | 无（空） |
| `0x003000` | `0x1000` | `0x0101` | `WIFI_CAL_RADIO1` | 二进制（TYFC0003） | 2G `rtl8192xbe` 校准组装 | 无（空） |
| `0x004000` | `0x0800` | `0x0110` | `BOSA_CALIBRATION` | 二进制 | `rtkbosa_k.bin`（1024B） | 无（空） |
| `0x004800` | `0x0400` | `0x0111` | `BOSA_OLT_CALIBRATION` | 二进制 | `rtkoltbosa_k.bin`（588B） | 无（空） |
| `0x005000` | `0x1000` | `0x0112` | `XTEMP_CALIBRATION` | JSON 文本 | `xtemp.cal` | 无（空） |
| `0x010000` | `0x7E0000` | `0x0200` | `FPGA_IMAGE` | 二进制 | `ubi_fpga` 卷原始内容（fpgafs squashfs，4317184B） | 无（空） |

`0x001040`–`0x001FFF`、`0x006000`–`0x00FFFF` 为预留空隙；新硬件字段优先使用 tag `0x0012`–`0x00FF`（身份区）与 `0x0102`–`0x010F` / `0x0113`–`0x011F`（校准区），并复用预留空隙的偏移。

### CRC 规则

统一使用 CRC32（多项式 `0xEDB88320`，反射）：

- `crc32(data)`：标准 zlib 式（初值 `0xFFFFFFFF`，最终异或），用于目录条目的数据 CRC。
- 寄存器式 `register(data, seed)`：无异或终值，用于三个头部字段：
  - `table_crc = register(dir, 0xFFFFFFFF)`；
  - `header_crc`：对 header 三段链式计算 `register(0x28..0x7C)` → `register(0x18..0x24)` → `register(0x00..0x14)`，覆盖 header 中除 `image_crc` 与 `header_crc` 自身外的全部字段；
  - `image_crc = register(image, 0xFFFFFFFF)`，计算时 `image_crc` 字段置 0。

### flash-calibration.bin（TYFC0003）

Wi-Fi 校准段为原厂 `factory.uc` 组装格式的逐字节移植，字段取自 `config_hs.xml`：

1. 魔数 `TYFC0003`（8B）。
2. 5 个 RX gaingap 单字节：`HW_RX_GAINGAP_{2G_CCK,2G_OFDM,5GL,5GM,5GH}_AB`，十进制字符串转字节。
3. 每 path A/B 的 `HW_THERMAL_{A,B}_WLAN{n}`（2B）与 `HW_XCAP_WLAN{n}_AX`（1B），十进制。
4. band × path 的 TSSI hex 数组：`HW_{2G_CCK,2G_BW40_1S,5G_BW40_1S}_TSSI_{A,B}`，逗号分隔 hex。
5. 6B MAC：BASE_MAC 整体加 `2 - radio_index`（5G wlan0 为 +2，2G wlan1 为 +1）。
6. `HW_2G_CCK_TSSI_{GAIN,CW}_DIFF_{A,B}` 与 `HW_{2G,5G}_TSSI_{GAIN,CW}_DIFF_{A,B}` hex 数组。
7. 尾部 112B：种子 `0` 与 `HW_5G_TSSI_CW_DIFF_A` 最后一字节，各重复 4 个 width × 14B，其中 width 1 全 0。

### 数据提取说明

- **U-Boot env**：`crc32(4B LE) + flags(1B) + data`；`data` 为 `name=value\0` 列表，CRC 校验 zlib 式。多分区（env/env2）均校验，取 flags 更大者。`kp_*` 即产线 keyparam。
- **ubi_device 容器**：按 EC 头 `vid_hdr_offset`/`data_offset` 解析，每 PEB 校验 EC/VID CRC，同 `(vol_id, lnum)` 取 `sqnum` 较新者；卷表（vol_id `0x7FFFEFFF`）定位卷名。dynamic 卷未映射（free）的 LEB 以 `0xFF` 补齐到 `reserved_pebs`，不影响 UBIFS 提交索引。
- **UBIFS**：从 superblock / master（LEB 1、2 取 commit 最新）沿 TNC 提交索引提取 `config_hs.xml`、`config.xml`、校准文件。压缩支持 none / LZO / zlib（raw deflate）。
- **ubi_fpga**：卷原始内容直接搬移（洞补 `0xFF`），消费端整体写回对应卷即可。

## U-Boot 刷入约定

bin 的内容整体写入 UBI static 卷 `factory`（`ubiupdatevol` 或 U-Boot `ubi write`）。恢复单字段时按目录条目的 `offset`/`size`/`crc32` 校验后读取。

## 扩展新机型

在 `src/profile.rs` 实现新的 `DeviceProfile`（env + ubi_device → 字段），`layout::FIELDS` 仅在需要新字段时追加 tag，`0x800000` 内预留空间充足。

## 已知限制

- `tyfza1` 无整机 SN 的产线数据源，`DEVICE_SN` 为空。
- LOID/PLOAM/OMCI 密钥、产品型号、省份、默认 SSID/密码等上层自定义数据不入 Factory bin，由上层软件从原配置自行管理。
- UBIFS zstd 压缩未支持（Realtek ASDK 5.10 镜像未使用）。
