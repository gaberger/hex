# Inference Tiers: T1, T2, and T2.5

Inference tiers are designed to optimize the performance and cost-effectiveness of machine learning model deployment. This guide explains the differences between T1, T2, and T2.5 tiers and when to use each.

## T1 Tier
The T1 tier is optimized for lightweight, low-latency inference tasks. It is best suited for scenarios where **scaffold** operations are required, such as generating simple templates or performing preliminary data processing. Use T1 when:
- Latency is critical
- The model is small and inference demands are minimal
- Costs need to be minimized for high-throughput use cases

## T2 Tier
The T2 tier is designed for moderate-complexity tasks, including **codegen** operations and intermediate-level inference. It balances performance and cost, making it ideal for tasks that require more computational power than T1 but do not justify the resources of T2.5. Use T2 when:
- The model requires moderate computational resources
- Tasks involve intermediate-level processing, such as code generation
- You need a balance between latency and cost

## T2.5 Tier
The T2.5 tier is optimized for high-complexity, resource-intensive inference tasks. It supports advanced operations and large models, making it suitable for scenarios where maximum accuracy and computational power are required. Use T2.5 when:
- The model is large and requires significant resources
- Tasks involve complex computations or high accuracy demands
- Latency is less critical than performance and precision

By selecting the appropriate tier for your use case, you can ensure optimal performance, efficiency, and cost-effectiveness for your inference tasks.