import type { Guid } from "guid-typescript";
import { writable } from "svelte/store";
import { Session } from "../models/session";

export const currentSession = writable(new Session('New Session'));
let sessions: Session[] = new Array<Session>();
let sessionCounter = 0;

function setCurrentSession(session: Session) {
    currentSession.set(session);
}

function setCurrentSessionById(id: Guid) {
    const session = sessions.find(session => session.id.equals(id));
    if (session) {
        setCurrentSession(session);
    }
}

function addSession(name: string) {
    sessionCounter++;
    const newSession = new Session(name);
    sessions.push(newSession);
    if (sessionCounter == 1) {
        setCurrentSession(newSession);
    }
}

function closeSession(id: Guid) {
    sessionCounter--;
    sessions = sessions.filter(session => !session.id.equals(id));
    if (sessionCounter == 1) {
        setCurrentSession(sessions[0]);
    }
}
