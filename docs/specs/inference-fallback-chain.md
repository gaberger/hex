# Behavioral Specifications for Inference Fallback Chain

## 1. ollama_endpoint_used_for_tools
- **Provider**: 'ollama'
- **Request**:
  - **Method**: POST
  - **URL**: `{url}/api/chat`
  - **Body**:
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

## 2. openrouter_endpoint_used_for_tools
- **Provider**: 'openrouter' OR URL contains 'openrouter.ai'
- **Request**:
  - **Method**: POST
  - **URL**: `{url}/chat/completions`
  - **Body** (OpenAI tools format)
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

## 3. openai_compat_endpoint_used_for_tools
- **Provider**: 'openai-compat' OR 'openai'
- **Request**:
  - **Method**: POST
  - **URL**: `{base}/chat/completions`
  - **Base URL Recognitions**:
    - `/v1`
    - `/openai`
    - `/v1beta/openai`
- **Body** (OpenAI tools format)