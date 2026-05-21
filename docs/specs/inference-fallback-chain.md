```markdown
# Inference Fallback Chain Specification

## Behavioral Specifications

### 1. ollama_endpoint_used_for_tools

- **Condition**: provider='ollama'
- **HTTP Method**: POST
- **URL**: {url}/api/chat
- **Request Body**:
  ```json
  {
    "tools": []
  }
  ```
- **Response Shape**:
  ```json
  {
    "message": {
      "tool_calls": []
    }
  }
  ```

### 2. openrouter_endpoint_used_for_tools

- **Condition**: provider='openrouter' OR url contains 'openrouter.ai'
- **HTTP Method**: POST
- **URL**: {url}/chat/completions
- **Request Body**:
  ```json
  {
    "tools": []
  }
  ```
- **Response Shape**:
  ```json
  {
    "choices": [
      {
        "message": {
          "tool_calls": []
        }
      }
    ]
  }
  ```

### 3. openai_compat_endpoint_used_for_tools

- **Condition**: provider='openai-compat' OR 'openai'
- **HTTP Method**: POST
- **URL**: {base}/chat/completions
  - Recognizes suffixes: /v1, /openai, /v1beta/openai
- **Request Body**:
  ```json
  {
    "tools": []
  }
  ```
- **Response Shape**:
  ```json
  {
    "choices": [
      {
        "message": {
          "tool_calls": []
        }
      }
    ]
  }
  ```
```