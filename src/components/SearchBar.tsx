interface SearchBarProps {
  query: string;
  onInput: (value: string) => void;
  filterActive: boolean;
  onFilterToggle: () => void;
}

export default function SearchBar(props: SearchBarProps) {
  return (
    <div class="search-container">
      <div class="search-row">
        <div class="search-input-wrapper">
          <svg class="search-icon" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M21 21l-6-6m2-5a7 7 0 11-14 0 7 7 0 0114 0z" />
          </svg>
          <input
            type="text"
            placeholder="Search transcriptions..."
            value={props.query}
            onInput={(e) => props.onInput(e.currentTarget.value)}
            class="search-input"
          />
        </div>
        <button
          class={`filter-toggle-btn ${props.filterActive ? "active" : ""}`}
          onClick={props.onFilterToggle}
          title="Filter by date & time"
        >
          <svg width="16" height="16" fill="none" stroke="currentColor" viewBox="0 0 24 24">
            <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M3 4a1 1 0 011-1h16a1 1 0 011 1v2.586a1 1 0 01-.293.707l-6.414 6.414a1 1 0 00-.293.707V17l-4 4v-6.586a1 1 0 00-.293-.707L3.293 7.293A1 1 0 013 6.586V4z" />
          </svg>
        </button>
      </div>
    </div>
  );
}
