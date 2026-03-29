import { fetchStandings } from '../ports/standings'
import { fetchRaceResults } from '../ports/race-results'
import { fetchConstructorTable } from '../ports/constructor-table'
import { fetchDriverProfiles } from '../ports/driver-profiles'
import { fetchHistoricalData } from '../ports/historical-data'

export async function main() {
  const command = process.argv[2]
  const args = process.argv.slice(3)

  switch (command) {
    case 'standings':
      await fetchStandings(args)
      break
    case 'race-results':
      await fetchRaceResults(args)
      break
    case 'constructor-table':
      await fetchConstructorTable(args)
      break
    case 'driver-profiles':
      await fetchDriverProfiles(args)
      break
    case 'historical-data':
      await fetchHistoricalData(args)
      break
    default:
      console.error('Invalid command. Available commands: standings, race-results, constructor-table, driver-profiles, historical-data')
      process.exit(1)
  }
}

main()