import React, { useState } from 'react';

interface DurationPickerProps {
  onSelect: (duration: number) => void; // Duration in seconds
}

const DurationPicker: React.FC<DurationPickerProps> = ({ onSelect }) => {
  const [duration, setDuration] = useState<number>(3600); // Default to 1 hour

  const handleSelect = (e: React.ChangeEvent<HTMLSelectElement>) => {
    const selectedValue = parseInt(e.target.value, 10);
    setDuration(selectedValue);
    onSelect(selectedValue);
  };

  return (
    <div>
      <label htmlFor="duration">Listing Duration:</label>
      <select id="duration" value={duration} onChange={handleSelect}>
        <option value={60}>1 Minute</option>
        <option value={3600}>1 Hour</option>
        <option value={86400}>1 Day</option>
        <option value={604800}>7 Days (1 Week)</option>
        <option value={2592000}>30 Days (1 Month)</option>
      </select>
    </div>
  );
};

export default DurationPicker;

// Referencing the spec that outlines the duration picker requirements
// docs/specs/ebay-spec-006