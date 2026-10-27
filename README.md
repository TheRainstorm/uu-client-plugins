# Eclipse Remote 插件

本仓库独立维护 Eclipse Remote 的构建时协议插件。主项目通过 `plugins` Git 子模块固定本仓库的提交；插件依赖主项目的 Rust SDK 与 workspace 配置，当前不提供热加载 DLL 或稳定二进制 ABI。

- `uu-compat`：UU 账号、设备注册、连接服务、兼容控制端，以及默认关闭的实验性官方客户端被控适配。
- 原生设备身份、配对、采集、编解码、输入所有权和系统后端由 [uu-client](https://github.com/TheRainstorm/uu-client) 维护，插件通过受限接口调用。

## 开发与验证

```sh
git clone --recurse-submodules https://github.com/TheRainstorm/uu-client.git
cd uu-client
git -C plugins switch -c plugin-change
cargo test --release -p uu-compat --lib --locked
cargo check -p uu-gui -p uu-cli --all-targets --locked
```

插件 crate 作为主 workspace 的成员构建，版本与依赖从主项目继承。单独克隆本仓库不能直接运行 Cargo；请使用以上联合检出。插件 CI 固定已验证的引擎提交，SDK 变化时同时更新该绑定。修改插件后先提交并推送本仓库，再在主项目提交新的子模块指针；其他开发者运行 `git submodule update --init --recursive`。

禁用应用默认功能或设置 `ECLIPSE_UU_PLUGIN=0` 可关闭 UU 网络入口。应用仍引用插件的数据模型；核心不依赖插件。官方被控入口另有默认关闭的设置及独立键鼠、音频、剪贴板许可，不获得 Eclipse 管理权限。协议夹具通过与真实官方手机互通属于不同的验证范围，当前状态以主项目的版本说明为准。

## 来源与许可

原创代码可选 MIT 或 Apache-2.0，见随仓库提供的许可原文；保留文件内的第三方来源与许可说明。初始兼容实现迁自 uu-client，Git 历史保留对应迁移提交。Windows 注册资料采集脚本同属兼容插件。

独立仓库与插件边界不改变第三方许可义务，也不授予第三方服务接入、商标或编解码专利权。新增官方被控适配根据协议证据独立编写，没有复制 GPL 参考项目的实现。完整应用的依赖清单和随包通知由主项目根据锁文件生成。
