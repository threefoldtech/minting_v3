import React, { useEffect, useState } from 'react';
import axios from 'axios';
import './App.css';

function App() {
	const [receiptHash, setReceiptHash] = useState("");
	const [receipt, setReceipt] = useState(undefined);
	const [error, setError] = useState("");

	const handleInput = (e) => {
		const input = e.target.value
		if (/^[0-9a-f]+$/.test(input) || input === "") {
			setReceiptHash(input);
			setError("")
		}
    }

	const blockSubmit = (e) => {
		e.preventDefault();
	}

	useEffect(() => {
		if (receiptHash.length === 64) {
			axios.get(`https://alpha.minting.tfchain.grid.tf/api/v1/receipt/${receiptHash}`).then(res => {
				setError("")
				setReceipt(res.data)
			}).catch(e => {
				setError(`Receipt with hash ${receiptHash} not found`)
				setReceipt(undefined)
			})
		}
	}, [setReceipt,receiptHash])

  return (
    <div className="App">
      <header className="App-header">
		<h1>ALPHA SOFTWARE - ALL DATA SUBJECT TO CHANGE</h1>
		<form className="receiptForm" onSubmit={blockSubmit}>
			<label className="mintingInput">
				Receipt Hash: <input type="text" value={receiptHash} onChange={handleInput} className="receiptInput"/>
			</label>
	  {
		error && (
			<div className="error">{error}</div>
		)
	  }
		</form>
	  {
		receipt && receipt.Minting && (
			<table>
				<tbody>
					<tr><th></th><th>Node info</th></tr>
					<tr><th>Node ID</th><td>{receipt.Minting.node_id} ({receipt.Minting.node_type})</td></tr>
					<tr><th>Farm Name</th><td>{receipt.Minting.farm_name} ({receipt.Minting.farm_id})</td></tr>
					<tr><th>Measured Uptime</th><td>{(receipt.Minting.measured_uptime / (3600 * 24)).toFixed(2) } days ({(100 * receipt.Minting.measured_uptime / 2630880).toFixed(2)}%)</td></tr>
					<tr><th></th><th>Node Resources</th></tr>
					<tr><th>CU</th><td>{receipt.Minting.cloud_units.cu.toFixed(2)}</td></tr>
					<tr><th>SU</th><td>{receipt.Minting.cloud_units.su.toFixed(2)}</td></tr>
					<tr><th>NU</th><td>{receipt.Minting.cloud_units.nu.toFixed(2)}</td></tr>
					<tr><th>Cru</th><RenderCru receipt={receipt}/></tr>
					<tr><th>Mru</th><td>{receipt.Minting.resource_units.mru.toFixed(3)} GB</td></tr>
					<tr><th>Sru</th><td>{receipt.Minting.resource_units.sru.toFixed(3)} GB</td></tr>
					<tr><th>Hru</th><td>{receipt.Minting.resource_units.hru.toFixed(3)} GB</td></tr>
					<tr><th></th><th>Payout info</th></tr>
					<tr><th>TFT Farmed</th><td>{(receipt.Minting.reward.tft / 1e7).toFixed(7)} TFT ({(receipt.Minting.reward.musd / 1e3).toFixed(3)}$ at {(receipt.Minting.tft_connection_price / 1e3).toFixed(3)}$/TFT)</td></tr>
					<tr><th>Payout Address</th><td>{receipt.Minting.stellar_payout_address}</td></tr>
				</tbody>
			</table>
		) 
	  }
		{
			receipt && receipt.Retry && (
				<table>
					<tbody>
						<tr><th>Retry for receipt</th><td>{receipt.Retry.retry_for_receipt}</td></tr>
						<tr><th>Old stellar address</th><RenderStellarAddress address={receipt.Retry.previous_stellar_payout_address} /></tr>
						<tr><th>New stellar address</th><RenderStellarAddress address={receipt.Retry.stellar_payout_address} /></tr>
						<tr><th>Amount of TFT</th><td>{(receipt.Retry.reward.tft / 1e7).toFixed(7)} TFT</td></tr>
					</tbody>
				</table>
			)
		}
		{
			receipt && receipt.Fixup && (
				<table>
					<tbody>
						<tr><th></th><th>Node info</th></tr>
						<tr><th>Node ID</th><td>{receipt.Fixup.node_id}</td></tr>
						<tr><th>Farm ID</th><td>{receipt.Fixup.farm_id}</td></tr>
						<tr><th></th><th>Node Resources(previously minted | actual | deficit)</th></tr>
						<tr><th>CU</th><td>{receipt.Fixup.minted_cloud_units.cu.toFixed(2)}|{receipt.Fixup.correct_cloud_units.cu.toFixed(2)}|{receipt.Fixup.fixup_cloud_units.cu.toFixed(2)}</td></tr>
						<tr><th>SU</th><td>{receipt.Fixup.minted_cloud_units.su.toFixed(2)}|{receipt.Fixup.correct_cloud_units.su.toFixed(2)}|{receipt.Fixup.fixup_cloud_units.su.toFixed(2)}</td></tr>
						<tr><th>NU</th><td>{receipt.Fixup.minted_cloud_units.nu.toFixed(2)}|{receipt.Fixup.correct_cloud_units.nu.toFixed(2)}|{receipt.Fixup.fixup_cloud_units.nu.toFixed(2)}</td></tr>
						<tr><th></th><th>Payout info</th></tr>
						<tr><th>TFT Received</th><td>{(receipt.Fixup.minted_reward.tft / 1e7).toFixed(7)} TFT</td></tr>
						<tr><th>TFT Owed</th><td>{(receipt.Fixup.correct_reward.tft / 1e7).toFixed(7)} TFT</td></tr>
						<tr><th>Additional TFT minted</th><td>{(receipt.Fixup.fixup_reward.tft / 1e7).toFixed(7)} TFT</td></tr>
						<tr><th>Payout Address</th><td>{receipt.Fixup.stellar_payout_address}</td></tr>
					</tbody>
				</table>
			)
		}
      </header>
    </div>
  );
}

const RenderStellarAddress = ({address}) => {
	if (!address) {
		return (<td>&#9001;NOT SET&#9002;</td>)
	}
	return (<td>{address}</td>)
}

const RenderCru = ({receipt}) => {
	if (!receipt.Minting) return null
	if (receipt.Minting.resource_units.cru < 0.1) {
		return (<td>{(receipt.Minting.resource_units.cru * 1024**3).toFixed(0)} VCpu</td>)
	} else {
		return (<td>{receipt.Minting.resource_units.cru} VCpu</td>)
	}
}

export default App;
