const PRICE = 39.9;
let count = 0;

const qty = document.querySelector('#qty');
const button = document.querySelector('#add-to-cart');
const countEl = document.querySelector('#cart-count');
const totalEl = document.querySelector('#cart-total');

function render() {
  countEl.textContent = count;
  totalEl.textContent = (count * PRICE).toFixed(2).replace('.', ',') + ' €';
}

button.addEventListener('click', () => {
  count = count + (parseInt(qty.value, 10) || 0);
  render();
});

render();
