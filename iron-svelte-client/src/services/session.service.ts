import type {Guid} from "guid-typescript";
import type {Writable} from "svelte/store";
import {writable} from "svelte/store";
import {Session} from "../models/session";
import type {IRGUserInteraction} from '../../static/iron-remote-gui';

export const userInteractionService: Writable<IRGUserInteraction> = writable();
export const currentSession: Writable<Session> = writable();

let _currentSession: Session = new Session('NewSession');
let sessions: Session[] = new Array<Session>();
let sessionCounter = 0;

addSession('NewSession');

function setCurrentSession(session: Session) {
    currentSession.set(session);
    _currentSession = session;
}

export function getCurrentSession(): Session {
    return _currentSession;
}

export function setCurrentSessionActive(active: boolean) {
   currentSession.update(session => {
       session.active = true;
       return session;
   }); 
}

export function setCurrentSessionById(id: Guid) {
    const session = sessions.find(session => session.id.equals(id));
    if (session) {
        setCurrentSession(session);
    }
}

export function addSession(name: string) {
    sessionCounter++;
    const newSession = new Session(name);
    sessions.push(newSession);
    if (sessionCounter == 1) {
        setCurrentSession(newSession);
    }
}

export function closeSession(id: Guid) {
    sessionCounter--;
    sessions = sessions.filter(session => !session.id.equals(id));
    if (sessionCounter == 1) {
        setCurrentSession(sessions[0]);
    }
}
