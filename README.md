# hal

An agentic TUI coding assistant written in Rust.

## Features

- Terminal-based chat interface with syntax-highlighted diffs
- Built-in tools: file read/write/edit, directory listing, grep, bash execution
- Sandboxed command execution (macOS, Linux)
- Session persistence
- Works with any OpenAI-compatible API (Gemini, OpenAI, Groq, OpenRouter, Ollama, etc.)

## Build

```
cargo build --release
```

## Setup

Set your API key and run:

```
export HAL_API_KEY_GEMINI="your-api-key"
./target/release/hal
```

Gemini is the default provider. To use a different provider, edit `~/.config/hal/config.json`:

```json
{
  "default_provider": "openai",
  "mode": "coding",
  "providers": {
    "openai": {
      "base_url": "https://api.openai.com/v1",
      "model": "gpt-4o",
      "api_key_env": "HAL_API_KEY_OPENAI"
    },
    "ollama": {
      "base_url": "http://localhost:11434/v1",
      "model": "llama3",
      "api_key_env": "HAL_API_KEY_OLLAMA"
    }
  }
}
```

## License

MIT
