# GitHub API Setup

## 问题：403 Forbidden 错误

当使用 `add` 或 `scan` 命令时，如果遇到 "GitHub API error: 403 Forbidden" 错误，这通常是因为：

1. **未认证的 API 调用** - 未设置 GitHub token
2. **API 速率限制** - 未认证的每小时只能调用 60 次

## 解决方案

### 方案一：设置 GitHub Token（推荐）

1. **创建 GitHub Personal Access Token**
   - 访问 [GitHub Settings → Developer settings → Personal access tokens → Tokens (classic)](https://github.com/settings/tokens)
   - 点击 "Generate new token"
   - 选择适当的权限 scope，至少需要 `public_repo`
   - 生成 token 并保存（只会显示一次）

2. **设置环境变量**
   
   **临时设置（当前会话有效）：**
   ```bash
   export GITHUB_TOKEN=ghp_your_token_here
   ```
   
   **永久设置（推荐）：**
   
   在 `~/.bashrc`、`~/.zshrc` 或 `~/.profile` 中添加：
   ```bash
   export GITHUB_TOKEN=ghp_your_token_here
   ```
   
   然后重新加载配置文件：
   ```bash
   source ~/.bashrc  # 或 ~/.zshrc / ~/.profile
   ```

3. **使用 `.env` 文件**
   
   在项目根目录创建 `.env` 文件：
   ```env
   GITHUB_TOKEN=ghp_your_token_here
   ```

### 方案二：不使用 Token（限制使用）

如果不设置 token，仍然可以使用，但有以下限制：
- 每小时最多 60 次 API 调用
- 如果超过限制，需要等待重置（通常每小时）

## 测试设置

设置 token 后，可以测试是否正常工作：

```bash
# 设置 token
export GITHUB_TOKEN=your_token_here

# 运行程序
cargo run -- add octocat/Hello-World
```

## 关于速率限制

GitHub API 速率限制：
- **未认证**: 60 requests/hour
- **认证（token）**: 5000 requests/hour
- **认证（GHE）**: 5000 requests/hour
- **认证（+ OAuth）**: 5000 requests/hour

## 错误信息说明

程序会显示：
- 如果未设置 token：`Warning: No GITHUB_TOKEN set - rate limited to 60 requests/hour`
- 如果遇到速率限制：`GitHub API rate limit exceeded. Set GITHUB_TOKEN to increase limit`
- 如果 token 无效：`GitHub API access forbidden. Please check your access or set GITHUB_TOKEN`