# Contributing to Musictui2

Thank you for your interest in contributing to Musictui2! This document provides guidelines for contributing to the project.

## Code of Conduct

Please note that this project adopts a Code of Conduct. Participants are expected to follow it in all interactions.

## Development Setup

1. Fork the repository on GitHub
2. Clone your fork locally:
   ```bash
   git clone https://github.com/yourusername/musictui2.git
   cd musictui2
   ```
3. Add the upstream repository:
   ```bash
   git remote add upstream https://github.com/originalowner/musictui2.git
   ```

## Development Workflow

1. Create a feature branch from `main`:
   ```bash
   git checkout -b feature/your-feature-name
   ```

2. Make your changes and follow the coding guidelines

3. Ensure all tests pass:
   ```bash
   cargo test
   ```

4. Check formatting and linting:
   ```bash
   cargo fmt
   cargo clippy -- -D warnings
   ```

5. Commit your changes with a clear commit message:
   ```bash
   git commit -m "feat: add support for flac audio format"
   ```

6. Push to your fork and create a pull request

## Coding Guidelines

### Rust Style

- Follow Rust conventions and idioms
- Use `cargo fmt` for formatting
- Run `cargo clippy` with warnings as errors
- Keep functions small and focused
- Use meaningful variable and function names

### Error Handling

- Always handle errors explicitly
- Use `Result<T, E>` for fallible operations
- Provide user-friendly error messages in UI-facing code
- Log detailed error context on the server side

### Testing

- Write tests for new functionality
- Maintain at least 80% test coverage
- Use integration tests for complex scenarios
- Test both success and failure cases

### Commits

Use conventional commit format:
- `feat:` for new features
- `fix:` for bug fixes
- `docs:` for documentation changes
- `refactor:` for code restructuring
- `test:` for adding tests
- `chore:` for maintenance tasks

### Pull Requests

PRs should:
- Have a clear title and description
- Include any relevant issues
- Pass all CI checks
- Have tests for new functionality
- Be reviewed and approved before merging

## Project Structure

```
src/
├── main.rs              # Application entry point
├── lib.rs               # Library exports
├── models.rs            # Data models
├── audio/               # Audio playback
├── database/            # Database operations
├── github/              # GitHub API
├── tui/                 # Terminal UI
├── cli/                 # Command-line interface
├── cache/               # File caching
└── events/              # Event system
```

## Adding Features

When adding a new feature:

1. Create appropriate data models in `models.rs`
2. Implement business logic in the relevant module
3. Add CLI commands if needed
4. Update the TUI to support the new feature
5. Write tests for the new functionality
6. Update documentation

## Reporting Bugs

When reporting bugs, please include:

1. Steps to reproduce
2. Expected behavior
3. Actual behavior
4. Environment information (OS, Rust version, etc.)
5. Any error messages

## Feature Requests

We welcome feature requests! Please:

1. Check if the feature already exists
2. Create an issue with a clear description
3. Explain the use case
4. Suggest implementation if you have ideas

## Getting Help

If you need help:

1. Check existing issues and discussions
2. Create a new issue with appropriate labels
3. Be specific and provide as much context as possible

Thank you for contributing to Musictui2! 🎵