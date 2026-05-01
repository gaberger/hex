# Inference Tiers: T1, T2, and T2.5

Inference tiers are used to categorize and route different types of tasks based on their complexity and resource requirements. Below is an explanation of each tier and when to use them.

## T1 Tier

**Use Case:** Basic inference tasks with lightweight models.  
**Example:** Scaffolding tasks, such as generating simple templates or boilerplate code.  
**Characteristics:** 
- Low latency 
- Minimal computational resources 
- Simple model architectures

Use T1 tier for tasks that require quick responses and do not involve complex computations.

## T2 Tier

**Use Case:** Moderate inference tasks with medium complexity.  
**Example:** Code generation tasks, such as generating function implementations or snippets.  
**Characteristics:** 
- Moderate latency 
- Mid-range computational resources 
- Moderately complex model architectures

Use T2 tier for tasks that require more computational resources than T1 but are not as resource-intensive as T2.5.

## T2.5 Tier

**Use Case:** Advanced inference tasks with high complexity.  
**Example:** Complex code generation, such as generating entire modules or architectures.  
**Characteristics:** 
- Higher latency 
- Significant computational resources 
- Highly complex model architectures

Use T2.5 tier for tasks that demand substantial computational power and involve complex model predictions.

## When to Use Each Tier

- **T1:** Use for quick, lightweight tasks where speed and simplicity are key.
- **T2:** Use for moderate tasks that require a balance between speed and computational resources.
- **T2.5:** Use for complex tasks where accuracy and detail are prioritized over speed.

By selecting the appropriate inference tier, you can optimize resource usage and ensure efficient task execution.