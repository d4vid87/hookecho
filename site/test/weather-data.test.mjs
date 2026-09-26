import test from 'node:test';
import assert from 'node:assert/strict';
import { temperature, percent, currentPeriods, validLocation, alertState } from '../src/scripts/weather-data.js';

test('weather data preserves missing values, units, freshness, and alert failures', () => {
  assert.equal(temperature(32,'F','C'),'0°');
  assert.equal(temperature(-40,'C','F'),'-40°');
  assert.equal(temperature(null),'—');
  assert.equal(percent(null),'—');
  assert.equal(percent(0),'0%');
  assert.equal(percent(101),'—');
  const now = Date.parse('2026-09-26T12:00:00Z');
  assert.deepEqual(currentPeriods([{name:'Old',shortForecast:'Clear',startTime:'2026-09-26T10:00:00Z',endTime:'2026-09-26T11:00:00Z'}],now),[]);
  assert.throws(() => alertState(null));
  assert.throws(() => alertState([{}]));
  assert.deepEqual(alertState([],now),[]);
  const alert = {event:'Flood Warning',status:'Actual',messageType:'Alert',severity:'Severe',expires:'2026-09-26T14:00:00Z'};
  assert.equal(alertState([{properties:alert}],now).length,1);
  assert.equal(alertState([{properties:{...alert,status:'Test'}}],now).length,0);
  assert.equal(alertState([{properties:alert}],now+3*3600000).length,0);
  assert.equal(validLocation({label:'Test',lat:35,lon:-97,site:'KTLX'}),true);
  assert.equal(validLocation({label:'Test',lat:999,lon:-97,site:'KTLX'}),false);
  assert.equal(validLocation({label:'Test',lat:35,lon:-97,site:'<script>'}),false);
});
