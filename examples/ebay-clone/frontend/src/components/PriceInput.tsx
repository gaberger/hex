import React, { useState } from 'react';

// PriceInput component for handling price input formatted as USD but stored as integer cents
const PriceInput: React.FC<{ onPriceChange: (cents: number) => void }> = ({ onPriceChange }) => {
  const [price, setPrice] = useState<string>('');

  const handleChange = (event: React.ChangeEvent<HTMLInputElement>) => {
    const value = event.target.value;
    // Allow only digits and at most one decimal point
    if (/^\d*\.?\d{0,2}$/.test(value)) {
      setPrice(value);
      // Convert to cents and call onPriceChange callback
      const cents = Math.round(parseFloat(value) * 100);
      onPriceChange(cents);
    }
  };

  return (
    <div>
      <label htmlFor="price">Starting Price (USD):</label>
      <input
        type="text"
        id="price"
        value={price}
        onChange={handleChange}
        placeholder="$0.00"
      />
    </div>
  );
};

export default PriceInput;
// Referenced spec: docs/specs/ebay-spec-006