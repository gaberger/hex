import { render } from 'solid-js/web';
import './dashboard.css';
import App from './App';

const root = document.getElementById('solid-root');
if (root) {
  render(() => <App />, root);
}
