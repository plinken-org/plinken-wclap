// PATCH (plinken-org): import from the wclap-host-js submodule via the Vite alias.
// Original: import {runThread} from "./wclap-js/wclap.mjs";
import {runThread} from "@webclap/wclap-host-js";

export function hostImports() {
	// imports for our particular host go here
	return {
		env: {
			eventsOutTryPush: (pluginPtr, ptr, length) => {
				throw Error("eventsOutTryPush");
			},
			webviewSend: (pluginPtr, ptr, length) => {
				throw Error("webviewSend");
			},
			stateMarkDirty: (pluginPtr) => {
				throw Error("stateMarkDirty");
			},
			paramsRescan: (pluginPtr, flags) => {
				throw Error("paramsRescan");
			},
			log: (pluginPtr, severity, msgPtr, length) => {
				// From here, we can't find the appropriate memory to read bytes from
				console.error("The plugin used clap.log!  It's not very effective...");
			}
		}
	};
};

export function startThreadWorker(host, wclapInit, threadData) {
	let name = `WCLAP instance 0x${threadData.instancePtr.toString(16)} thread #${threadData.threadId}`;
	console.log(`Starting Worker for ${name}`);
	// Load this module as a Worker
	let worker = new Worker(import.meta.url, {type: 'module', name: name});
	let data = host.getWorkerData(wclapInit, threadData);
	data.threadName = name;
	worker.postMessage(data);
	return worker;
}

if (globalThis.DedicatedWorkerGlobalScope) {
	addEventListener('message', async e => {
		let data = e.data;
		console.log(`Thread ready: ${data.threadName}`);
		try {
			await runThread(e.data, hostImports(), startThreadWorker);
			console.log(`Thread finished: ${data.threadName}`);
		} catch (e) {
			console.error(`Thread crashed: ${data.threadName}`, e);
		}
		close();
	});
}
