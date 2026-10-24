# GitHub Actions 打包说明

- 入口：`build`（`.github/workflows/build.yaml`）。
  - 推送 `v1.2.3` 形式的 tag：Linux（amd64/arm64v8/armv7/i386）、Windows x86_64 二进制及 deb 包发布到草稿 Release。
  - Actions 页面手动运行：只构建，产物在运行页面的 Artifacts 下载。
- Docker Hub / 上游 GHCR 镜像推送已关闭（需要本仓库没有的密钥）；本地镜像用 `docker/Dockerfile` 自行构建。
