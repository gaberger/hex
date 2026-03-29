export interface Todo {
  id: string
  title: string
  description?: string
  completed?: boolean
}

export const isTodoValid = (todo: Todo): boolean => {
  if (!todo.id) {
    return false
  }

  if (!todo.title) {
    return false
  }

  if (todo.title.length > 200) {
    return false
  }

  if (todo.description !== undefined && todo.description.length > 500) {
    return false
  }

  if (todo.completed !== undefined && !todo.completed && typeof todo.completed !== 'boolean') {
    return false
  }

  return true
}

export const validateTodo = (input: {
  id?: string
  title: string
  description?: string
  completed?: boolean
}): Todo => {
  const id = input.id || String(Date.now())
  const title = input.title.trim()
  const description = input.description?.trim()
  const completed = input.completed !== undefined ? input.completed : false

  const validated: Todo = {
    id,
    title,
    description,
    completed
  }

  const isValid = isTodoValid(validated)

  if (!isValid) {
    throw new Error('Invalid Todo entity')
  }

  return validated
}

export const TodoValidationError = class extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'TodoValidationError'
  }
}

export const assertTodoValid = (todo: Todo): void => {
  if (!isTodoValid(todo)) {
    throw new TodoValidationError('Todo validation failed')
  }
}