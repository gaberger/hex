import React, { useState, useEffect } from 'react';
import debounce from 'lodash.debounce';

interface SearchBarProps {
  onSearch: (query: string) => void;
}

const SearchBar: React.FC<SearchBarProps> = ({ onSearch }) => {
  const [searchTerm, setSearchTerm] = useState('');

  // Debounce the search function call
  const debouncedOnSearch = debounce(onSearch, 250);

  useEffect(() => {
    // Call the debounced search function whenever searchTerm changes
    if (searchTerm) {
      debouncedOnSearch(searchTerm);
    }
  }, [searchTerm, debouncedOnSearch]);

  return (
    <input
      type="text"
      value={searchTerm}
      onChange={(e) => setSearchTerm(e.target.value)}
      placeholder="Search auctions..."
      className="border p-2 rounded-full w-full"
    />
  );
};

export default SearchBar;
// docs/specs/ebay-spec-019