```markdown
# Inference Fallback Chain Specification

## BehavioralSpec[] Entries

### 1. ollama_endpoint_used_for_tools

- **Description**: When the provider is 'ollama', route the request to the specified URL with a POST method.
- **Request**:
  - **Method**: POST
  - **URL**: `{url}/api/chat`
  - **Payload**:
    ```json
    {
      "tools": []
    }
    ```
- **Response**:
  - **Shape**: `message.tool_calls`

### 2. openrouter_endpoint_used_for_tools

- **Description**: When the provider is 'openrouter' or the URL contains 'openrouter.ai', route the request to the specified URL with a POST method.
- **Request**:
  - **Method**: POST
  - **URL**: `{url}/chat/completions`
  - **Payload** (OpenAI tools format):
    ```json
    {
      "messages": []
    }
    ```
- **Response**:
  - **Shape**: `choices[0].message.tool_calls`

### 3. openai_compat_endpoint_used_for_tools

- **Description**: When the provider is 'openai-compat' or 'openai', route the request to the specified base URL with a POST method.
- **Request**:
  - **Method**: POST
  - **URL**: `{base}/chat/completions`
  - **Payload (OpenAI tools format)**:
    ```json
    {
      "messages": []
    }
    ```
- **Response**:
  - **Shape**: `choices[0].message.tool_calls`

## Implementation Notes

- The function `call_inference_endpoint_with_tools` in `hex-nexus/src/routes/chat.rs` is responsible for routing requests based on the provider and URL.
- The `priority_for_tools` function determines the order of preference among the three providers.
```