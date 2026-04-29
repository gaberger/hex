# Inference Tiers Guide

Inference tiers are designed to provide different levels of performance and cost-efficiency for various use cases. The available tiers are T1, T2, and T2.5. Each tier is optimized for specific types of tasks.

## T1 Tier

The T1 tier is ideal for basic inference tasks that require a balance between cost and performance. It is suitable for applications where response time can be slightly longer, but cost optimization is crucial. The T1 tier is often used for scaffolding projects or initial testing phases.

### Use Cases:
- Scaffolding new projects
- Initial testing of models
- Non-time-sensitive inference tasks

## T2 Tier

The T2 tier offers a good balance between performance and cost. It is designed for applications that require faster response times than the T1 tier but do not need the highest level of performance offered by the T2.5 tier. The T2 tier is commonly used for code generation tasks where speed is important but does not have to be the absolute fastest.

### Use Cases:
- Code generation
- General-purpose inference tasks
- Applications requiring faster response times than T1

## T2.5 Tier

The T2.5 tier provides the highest level of performance among the tiers. It is designed for applications that require the fastest possible inference times, such as real-time applications or those with strict latency requirements. The T2.5 tier is typically used in production environments where performance is critical.

### Use Cases:
- Real-time applications
- Production environments requiring high performance
- Applications with strict latency requirements

By choosing the appropriate inference tier, you can optimize both the cost and performance of your application based on its specific needs.