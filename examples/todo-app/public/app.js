const API = '/api';
let currentFilter = 'all';

// DOM refs
const form = document.getElementById('add-form');
const titleInput = document.getElementById('title-input');
const priorityInput = document.getElementById('priority-input');
const todoList = document.getElementById('todo-list');
const healthDot = document.getElementById('health-dot');
const healthText = document.getElementById('health-text');

// API helpers
async function api(path, options = {}) {
  const res = await fetch(`${API}${path}`, {
    headers: { 'Content-Type': 'application/json' },
    ...options,
  });
  if (res.status === 204) return null;
  const data = await res.json();
  if (!res.ok) throw new Error(data.error || 'Request failed');
  return data;
}

// Render
function renderTodo(todo) {
  const li = document.createElement('li');
  li.className = `todo-item${todo.status === 'completed' ? ' completed' : ''}`;
  li.dataset.id = todo.id;

  const checkbox = document.createElement('div');
  checkbox.className = 'todo-checkbox';
  checkbox.addEventListener('click', () => toggleComplete(todo));

  const body = document.createElement('div');
  body.className = 'todo-body';

  const title = document.createElement('div');
  title.className = 'todo-title';
  title.textContent = todo.title;

  const meta = document.createElement('div');
  meta.className = 'todo-meta';

  const badge = document.createElement('span');
  badge.className = `priority-badge priority-${todo.priority}`;
  badge.textContent = todo.priority;
  meta.appendChild(badge);

  if (todo.tags) {
    for (const t of todo.tags) {
      const tag = document.createElement('span');
      tag.className = 'tag';
      tag.textContent = t;
      meta.appendChild(tag);
    }
  }

  body.appendChild(title);
  body.appendChild(meta);

  const del = document.createElement('button');
  del.className = 'todo-delete';
  del.textContent = '\u00d7';
  del.title = 'Delete';
  del.addEventListener('click', () => deleteTodo(todo.id));

  li.appendChild(checkbox);
  li.appendChild(body);
  li.appendChild(del);
  return li;
}

function clearChildren(el) {
  while (el.firstChild) {
    el.removeChild(el.firstChild);
  }
}

async function refresh() {
  try {
    const filterParam = currentFilter === 'all' ? '' : `?status=${currentFilter}`;
    const [todos, stats] = await Promise.all([
      api(`/todos${filterParam}`),
      api('/stats'),
    ]);

    document.getElementById('stat-total').textContent = stats.total;
    document.getElementById('stat-pending').textContent = stats.pending;
    document.getElementById('stat-completed').textContent = stats.completed;
    document.getElementById('stat-rate').textContent = Math.round(stats.rate * 100) + '%';

    clearChildren(todoList);
    if (todos.length === 0) {
      const empty = document.createElement('div');
      empty.className = 'empty-state';
      empty.textContent = currentFilter === 'all'
        ? 'No todos yet. Add one above!'
        : `No ${currentFilter} todos.`;
      todoList.appendChild(empty);
    } else {
      for (const todo of todos) {
        todoList.appendChild(renderTodo(todo));
      }
    }

    healthDot.className = 'health-dot';
    healthText.textContent = 'Connected';
  } catch (err) {
    healthDot.className = 'health-dot error';
    healthText.textContent = 'Error';
    console.error('Refresh failed:', err);
  }
}

async function addTodo(e) {
  e.preventDefault();
  const title = titleInput.value.trim();
  if (!title) return;
  try {
    await api('/todos', {
      method: 'POST',
      body: JSON.stringify({
        title,
        priority: priorityInput.value,
      }),
    });
    titleInput.value = '';
    await refresh();
  } catch (err) {
    alert(err.message);
  }
}

async function toggleComplete(todo) {
  if (todo.status === 'completed') return;
  try {
    await api(`/todos/${todo.id}/complete`, { method: 'POST' });
    await refresh();
  } catch (err) {
    alert(err.message);
  }
}

async function deleteTodo(id) {
  try {
    await api(`/todos/${id}`, { method: 'DELETE' });
    await refresh();
  } catch (err) {
    alert(err.message);
  }
}

// Health check polling
async function checkHealth() {
  try {
    await api('/health');
    healthDot.className = 'health-dot';
    healthText.textContent = 'Connected';
  } catch {
    healthDot.className = 'health-dot error';
    healthText.textContent = 'Disconnected';
  }
}

// Filter buttons
document.getElementById('filters').addEventListener('click', (e) => {
  const btn = e.target.closest('.filter-btn');
  if (!btn) return;
  document.querySelectorAll('.filter-btn').forEach((b) => b.classList.remove('active'));
  btn.classList.add('active');
  currentFilter = btn.dataset.filter;
  refresh();
});

// Init
form.addEventListener('submit', addTodo);
refresh();
setInterval(checkHealth, 30000);
