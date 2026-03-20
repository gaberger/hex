//! OpenAPI 3.1 spec endpoint for the stateless compute routes.
//!
//! Serves GET /api/openapi.json with the API schema.
//! Only stateless routes are included — deprecated state-read routes are omitted.

use axum::Json;
use serde_json::{json, Value};

/// GET /api/openapi.json — serve OpenAPI 3.1 spec for stateless routes.
pub async fn openapi_spec() -> Json<Value> {
    Json(json!({
        "openapi": "3.1.0",
        "info": {
            "title": "hex-nexus API",
            "description": "Stateless compute layer for hex agent control plane. State reads should use SpacetimeDB direct subscriptions.",
            "version": env!("CARGO_PKG_VERSION"),
            "contact": { "name": "hex" }
        },
        "servers": [
            { "url": "http://localhost:5555", "description": "Local hex-nexus" }
        ],
        "paths": {
            "/api/analyze": {
                "post": {
                    "summary": "Run architecture analysis",
                    "tags": ["compute"],
                    "requestBody": {
                        "content": { "application/json": { "schema": { "type": "object", "properties": { "path": { "type": "string" } } } } }
                    },
                    "responses": { "200": { "description": "Analysis result" } }
                }
            },
            "/api/agents/spawn": {
                "post": {
                    "summary": "Spawn a new hex-agent process",
                    "tags": ["agents"],
                    "requestBody": {
                        "content": { "application/json": { "schema": {
                            "type": "object",
                            "required": ["projectDir"],
                            "properties": {
                                "projectDir": { "type": "string", "description": "Absolute path to project" },
                                "model": { "type": "string", "description": "LLM model override" },
                                "agentName": { "type": "string", "description": "Agent type: hex-coder, planner, tester, etc." }
                            }
                        } } }
                    },
                    "responses": {
                        "200": { "description": "Agent spawned" },
                        "503": { "description": "Agent manager not initialized" }
                    }
                }
            },
            "/api/agents/{id}": {
                "delete": {
                    "summary": "Terminate an agent",
                    "tags": ["agents"],
                    "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
                    "responses": { "200": { "description": "Agent terminated" } }
                }
            },
            "/api/fleet/register": {
                "post": {
                    "summary": "Register a fleet compute node",
                    "tags": ["fleet"],
                    "requestBody": {
                        "content": { "application/json": { "schema": { "type": "object", "properties": { "hostname": { "type": "string" } } } } }
                    },
                    "responses": { "200": { "description": "Node registered" } }
                }
            },
            "/api/inference/register": {
                "post": {
                    "summary": "Register an inference provider",
                    "tags": ["inference"],
                    "requestBody": {
                        "content": { "application/json": { "schema": {
                            "type": "object",
                            "properties": {
                                "name": { "type": "string" },
                                "provider_type": { "type": "string", "enum": ["ollama", "vllm", "openai", "anthropic", "llama-cpp"] },
                                "base_url": { "type": "string" },
                                "models": { "type": "array", "items": { "type": "string" } }
                            }
                        } } }
                    },
                    "responses": { "200": { "description": "Provider registered" } }
                }
            },
            "/api/workplan/execute": {
                "post": {
                    "summary": "Execute a workplan with swarm agents",
                    "tags": ["compute"],
                    "responses": { "200": { "description": "Workplan started" } }
                }
            },
            "/api/swarms": {
                "post": {
                    "summary": "Create a swarm (writes to SpacetimeDB)",
                    "tags": ["swarms"],
                    "responses": { "201": { "description": "Swarm created" } }
                }
            },
            "/api/hexflo/memory": {
                "post": {
                    "summary": "Store key-value in HexFlo memory",
                    "tags": ["hexflo"],
                    "responses": { "200": { "description": "Stored" } }
                }
            },
            "/api/openapi.json": {
                "get": {
                    "summary": "This OpenAPI spec",
                    "tags": ["meta"],
                    "responses": { "200": { "description": "OpenAPI 3.1 JSON" } }
                }
            }
        },
        "tags": [
            { "name": "compute", "description": "Stateless compute operations (analyze, summarize, scaffold)" },
            { "name": "agents", "description": "Agent process management (spawn, terminate)" },
            { "name": "fleet", "description": "Remote compute node management" },
            { "name": "inference", "description": "Inference provider registration and health" },
            { "name": "swarms", "description": "Swarm lifecycle (write operations only)" },
            { "name": "hexflo", "description": "HexFlo coordination memory (write operations only)" },
            { "name": "meta", "description": "API metadata" }
        ],
        "x-deprecated-routes": {
            "note": "The following GET routes are deprecated. Use SpacetimeDB direct subscriptions instead.",
            "routes": [
                "GET /api/swarms/active → spacetimedb://hexflo-coordination/SELECT * FROM swarm",
                "GET /api/agents → spacetimedb://agent-registry/SELECT * FROM agent",
                "GET /api/inference/endpoints → spacetimedb://inference-gateway/SELECT * FROM inference_provider",
                "GET /api/coordination/* → spacetimedb://hexflo-coordination/*",
                "GET /api/sessions/* → spacetimedb://chat-relay/SELECT * FROM conversation"
            ]
        }
    }))
}
