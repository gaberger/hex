import { CompositionRoot } from './composition-root.js';

const app = CompositionRoot.createDefault();
const html = await app.renderPage('main');

document.getElementById('app')!.innerHTML = html;

document.querySelectorAll('.hero-cta').forEach(el => {
  el.addEventListener('click', (e) => {
    e.preventDefault();
    console.log('CTA clicked');
  });
});

document.querySelectorAll('.card').forEach(el => {
  el.addEventListener('click', () => {
    const id = el.getAttribute('data-id');
    console.log('Card clicked:', id);
  });
});
